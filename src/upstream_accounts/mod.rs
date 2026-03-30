use super::*;
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
const DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE: usize = 20;
const UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS: [usize; 3] = [20, 50, 100];
const POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES: i64 = 30;
const POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD: i64 = 5;
const POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS: i64 = 30;
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
    forward_proxy_nodes: Vec<ForwardProxyBindingNodeResponse>,
    has_ungrouped_accounts: bool,
    routing: PoolRoutingSettingsResponse,
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
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
}

#[derive(Debug, Clone, Default)]
struct UpstreamAccountGroupMetadata {
    note: Option<String>,
    bound_proxy_keys: Vec<String>,
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
}

#[derive(Debug, Clone, Default)]
struct RequestedGroupMetadataChanges {
    note: Option<String>,
    note_was_requested: bool,
    bound_proxy_keys: Vec<String>,
    bound_proxy_keys_was_requested: bool,
}

impl RequestedGroupMetadataChanges {
    fn was_requested(&self) -> bool {
        self.note_was_requested || self.bound_proxy_keys_was_requested
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedRequiredGroupProxyBinding {
    pub(crate) group_name: String,
    pub(crate) bound_proxy_keys: Vec<String>,
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
    conversations: Vec<AccountStickyKeyConversation>,
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
    note: Option<String>,
    group_note: Option<String>,
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
    note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    group_note: OptionalField<String>,
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
    note: Option<String>,
    group_note: Option<String>,
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
    group_note: Option<String>,
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
enum ImportedOauthValidationTerminalEvent {
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
struct ImportedOauthValidationJob {
    target_group_name: String,
    target_bound_proxy_keys: Vec<String>,
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
enum BulkUpstreamAccountSyncTerminalEvent {
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
struct BulkUpstreamAccountSyncJob {
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
    note: Option<String>,
    group_note: Option<String>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTagRequest {
    name: String,
    guard_enabled: bool,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: bool,
    allow_cut_in: bool,
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
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountGroupRequest {
    note: Option<String>,
    #[serde(default)]
    bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    upstream_429_retry_enabled: Option<bool>,
    #[serde(default)]
    upstream_429_max_retries: Option<u8>,
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
    _last_refresh: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug, Clone)]
struct NormalizedImportedOauthCredentials {
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
struct ImportedOauthProbeOutcome {
    token_expires_at: String,
    credentials: StoredOauthCredentials,
    claims: ChatgptJwtClaims,
    usage_snapshot: Option<NormalizedUsageSnapshot>,
    exhausted: bool,
    usage_snapshot_warning: Option<String>,
}

#[derive(Debug, Clone)]
struct ImportedOauthValidatedImportData {
    normalized: NormalizedImportedOauthCredentials,
    probe: ImportedOauthProbeOutcome,
}

struct PersistOauthCallbackInput {
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

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct UpstreamAccountRow {
    id: i64,
    kind: String,
    provider: String,
    display_name: String,
    group_name: Option<String>,
    is_mother: i64,
    note: Option<String>,
    status: String,
    enabled: i64,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    plan_type: Option<String>,
    plan_type_observed_at: Option<String>,
    masked_api_key: Option<String>,
    encrypted_credentials: Option<String>,
    token_expires_at: Option<String>,
    last_refreshed_at: Option<String>,
    last_synced_at: Option<String>,
    last_successful_sync_at: Option<String>,
    last_activity_at: Option<String>,
    last_error: Option<String>,
    last_error_at: Option<String>,
    last_action: Option<String>,
    last_action_source: Option<String>,
    last_action_reason_code: Option<String>,
    last_action_reason_message: Option<String>,
    last_action_http_status: Option<i64>,
    last_action_invoke_id: Option<String>,
    last_action_at: Option<String>,
    last_selected_at: Option<String>,
    last_route_failure_at: Option<String>,
    last_route_failure_kind: Option<String>,
    cooldown_until: Option<String>,
    consecutive_route_failures: i64,
    temporary_route_failure_streak_started_at: Option<String>,
    compact_support_status: Option<String>,
    compact_support_observed_at: Option<String>,
    compact_support_reason: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
    upstream_base_url: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, FromRow)]
struct PoolRoutingSettingsRow {
    encrypted_api_key: Option<String>,
    masked_api_key: Option<String>,
    primary_sync_interval_secs: Option<i64>,
    secondary_sync_interval_secs: Option<i64>,
    priority_available_account_cap: Option<i64>,
    responses_first_byte_timeout_secs: Option<i64>,
    compact_first_byte_timeout_secs: Option<i64>,
    responses_stream_timeout_secs: Option<i64>,
    compact_stream_timeout_secs: Option<i64>,
    default_first_byte_timeout_secs: Option<i64>,
    upstream_handshake_timeout_secs: Option<i64>,
    request_read_timeout_secs: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PoolRoutingMaintenanceSettings {
    primary_sync_interval_secs: u64,
    secondary_sync_interval_secs: u64,
    priority_available_account_cap: usize,
}

impl PoolRoutingMaintenanceSettings {
    fn into_response(self) -> PoolRoutingMaintenanceSettingsResponse {
        PoolRoutingMaintenanceSettingsResponse {
            primary_sync_interval_secs: self.primary_sync_interval_secs,
            secondary_sync_interval_secs: self.secondary_sync_interval_secs,
            priority_available_account_cap: self.priority_available_account_cap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaintenanceTier {
    Priority,
    Secondary,
}

#[derive(Debug, Clone, FromRow)]
struct MaintenanceCandidateRow {
    id: i64,
    status: String,
    last_synced_at: Option<String>,
    last_error_at: Option<String>,
    token_expires_at: Option<String>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaintenanceDispatchPlan {
    account_id: i64,
    tier: MaintenanceTier,
    sync_interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum OptionalField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

fn deserialize_optional_field<'de, D, T>(deserializer: D) -> Result<OptionalField<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    if raw.is_null() {
        return Ok(OptionalField::Null);
    }

    serde_json::from_value(raw)
        .map(OptionalField::Value)
        .map_err(serde::de::Error::custom)
}

#[derive(Debug, FromRow)]
#[allow(dead_code)]
struct PoolStickyRouteRow {
    sticky_key: String,
    account_id: i64,
    created_at: String,
    updated_at: String,
    last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct AccountRoutingCandidateRow {
    id: i64,
    plan_type: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<String>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
    last_selected_at: Option<String>,
    active_sticky_conversations: i64,
    #[sqlx(default)]
    in_flight_reservations: i64,
}

impl AccountRoutingCandidateRow {
    fn effective_load(&self) -> i64 {
        self.active_sticky_conversations
            .saturating_add(self.in_flight_reservations.max(0))
    }

    fn capacity_profile(&self) -> RoutingCapacityProfile {
        let signals = self.window_signals();
        if signals.short_signal {
            RoutingCapacityProfile {
                soft_limit: 2,
                hard_cap: 3,
            }
        } else if signals.long_signal {
            RoutingCapacityProfile {
                soft_limit: 1,
                hard_cap: 2,
            }
        } else {
            RoutingCapacityProfile {
                soft_limit: 2,
                hard_cap: 3,
            }
        }
    }

    fn normalized_window_pressure(&self, now: DateTime<Utc>) -> NormalizedRoutingPressure {
        let mut short_pressure = None;
        let mut long_pressure = None;
        for window in [
            routing_window_state(
                self.primary_used_percent,
                self.primary_window_minutes,
                self.primary_resets_at.as_deref(),
                now,
                RoutingWindowBucket::Short,
            ),
            routing_window_state(
                self.secondary_used_percent,
                self.secondary_window_minutes,
                self.secondary_resets_at.as_deref(),
                now,
                RoutingWindowBucket::Long,
            ),
        ]
        .into_iter()
        .flatten()
        {
            match window.bucket {
                RoutingWindowBucket::Short => {
                    short_pressure = Some(short_pressure.unwrap_or(0.0_f64).max(window.pressure));
                }
                RoutingWindowBucket::Long => {
                    long_pressure = Some(long_pressure.unwrap_or(0.0_f64).max(window.pressure));
                }
            }
        }
        NormalizedRoutingPressure {
            short_pressure,
            long_pressure,
        }
    }

    fn window_signals(&self) -> RoutingWindowSignals {
        let mut short_signal = false;
        let mut long_signal = false;
        for window_minutes in [self.primary_window_minutes, self.secondary_window_minutes]
            .into_iter()
            .flatten()
        {
            if window_minutes <= 360 {
                short_signal = true;
            } else {
                long_signal = true;
            }
        }
        if self.primary_window_minutes.is_none()
            && (self.primary_used_percent.is_some() || self.local_primary_limit.is_some())
        {
            short_signal = true;
        }
        if self.secondary_window_minutes.is_none()
            && (self.secondary_used_percent.is_some() || self.local_secondary_limit.is_some())
        {
            long_signal = true;
        }
        RoutingWindowSignals {
            short_signal,
            long_signal,
        }
    }

    fn scarcity_score(&self, now: DateTime<Utc>) -> f64 {
        let pressure = self.normalized_window_pressure(now);
        match (pressure.short_pressure, pressure.long_pressure) {
            (Some(short), Some(long)) => (0.65 * short) + (0.35 * long),
            (Some(short), None) => short,
            (None, Some(long)) => long,
            (None, None) => 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RoutingCapacityProfile {
    soft_limit: i64,
    hard_cap: i64,
}

#[derive(Debug, Clone, Copy)]
struct NormalizedRoutingPressure {
    short_pressure: Option<f64>,
    long_pressure: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
struct RoutingWindowSignals {
    short_signal: bool,
    long_signal: bool,
}

#[derive(Debug, Clone, Copy)]
struct RoutingWindowState {
    bucket: RoutingWindowBucket,
    pressure: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoutingWindowBucket {
    Short,
    Long,
}

fn routing_window_state(
    used_percent: Option<f64>,
    window_minutes: Option<i64>,
    resets_at: Option<&str>,
    now: DateTime<Utc>,
    default_bucket: RoutingWindowBucket,
) -> Option<RoutingWindowState> {
    let used_ratio = normalize_unit_ratio(used_percent? / 100.0);
    let (bucket, remaining_ratio) = if let Some(window_minutes) = window_minutes {
        let window_minutes = window_minutes.max(1);
        let bucket = if window_minutes <= 360 {
            RoutingWindowBucket::Short
        } else {
            RoutingWindowBucket::Long
        };
        let window_duration_secs = (window_minutes as f64) * 60.0;
        let remaining_ratio = resets_at
            .and_then(parse_rfc3339_utc)
            .map(|reset_at| {
                normalize_unit_ratio(
                    (reset_at - now).num_seconds().max(0) as f64 / window_duration_secs,
                )
            })
            .unwrap_or(1.0);
        (bucket, remaining_ratio)
    } else {
        (default_bucket, 1.0)
    };
    Some(RoutingWindowState {
        bucket,
        pressure: used_ratio * remaining_ratio,
    })
}

fn normalize_unit_ratio(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

#[derive(Debug, FromRow)]
struct AccountActiveConversationCountRow {
    account_id: i64,
    active_conversation_count: i64,
}

#[derive(Debug, Clone, FromRow)]
struct TagRow {
    name: String,
    guard_enabled: i64,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: i64,
    allow_cut_in: i64,
}

#[derive(Debug, Clone, FromRow)]
struct AccountTagRow {
    account_id: i64,
    tag_id: i64,
    name: String,
    guard_enabled: i64,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: i64,
    allow_cut_in: i64,
}

#[derive(Debug, Clone, FromRow)]
struct TagListRow {
    id: i64,
    name: String,
    guard_enabled: i64,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: i64,
    allow_cut_in: i64,
    updated_at: String,
    account_count: i64,
    group_count: i64,
}

#[derive(Debug, FromRow)]
struct StickyKeyAggregateRow {
    sticky_key: String,
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    created_at: String,
    last_activity_at: String,
}

#[derive(Debug, FromRow)]
struct AccountLastActivityRow {
    account_id: i64,
    last_activity_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct AccountWindowUsageRow {
    occurred_at: String,
    upstream_account_id: i64,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct AccountWindowUsageAccumulator {
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
}

impl AccountWindowUsageAccumulator {
    fn add_row(&mut self, row: &AccountWindowUsageRow) {
        self.request_count += 1;
        self.total_tokens += row.total_tokens.unwrap_or_default();
        self.total_cost += row.cost.unwrap_or_default();
        self.input_tokens += row.input_tokens.unwrap_or_default();
        self.output_tokens += row.output_tokens.unwrap_or_default();
        self.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
    }

    fn into_snapshot(self) -> RateWindowActualUsage {
        RateWindowActualUsage {
            request_count: self.request_count,
            total_tokens: self.total_tokens,
            total_cost: self.total_cost,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_input_tokens: self.cache_input_tokens,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct AccountWindowUsagePlan {
    primary: Option<AccountWindowUsageRange>,
    secondary: Option<AccountWindowUsageRange>,
}

#[derive(Debug, Clone)]
struct AccountWindowUsageRange {
    start_at: String,
    end_at: String,
}

#[derive(Debug, Clone, Copy)]
struct AccountWindowUsageRangeBounds {
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
}

impl AccountWindowUsageRangeBounds {
    fn into_range(self) -> AccountWindowUsageRange {
        AccountWindowUsageRange {
            start_at: format_naive(self.start_at.with_timezone(&Shanghai).naive_local()),
            end_at: format_naive(self.end_at.with_timezone(&Shanghai).naive_local()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct AccountWindowUsageSummary {
    primary: AccountWindowUsageAccumulator,
    secondary: AccountWindowUsageAccumulator,
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamAccountActionEventRow {
    id: i64,
    occurred_at: String,
    action: String,
    source: String,
    reason_code: Option<String>,
    reason_message: Option<String>,
    http_status: Option<i64>,
    failure_kind: Option<String>,
    invoke_id: Option<String>,
    sticky_key: Option<String>,
    created_at: String,
}

#[derive(Debug, FromRow)]
struct StickyKeyEventRow {
    occurred_at: String,
    status: String,
    request_tokens: i64,
    sticky_key: String,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct UpstreamAccountSampleRow {
    captured_at: String,
    limit_id: Option<String>,
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<String>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct OauthLoginSessionRow {
    login_id: String,
    account_id: Option<i64>,
    display_name: Option<String>,
    group_name: Option<String>,
    group_bound_proxy_keys_json: Option<String>,
    is_mother: i64,
    note: Option<String>,
    tag_ids_json: Option<String>,
    group_note: Option<String>,
    mailbox_session_id: Option<String>,
    mailbox_address: Option<String>,
    state: String,
    pkce_verifier: String,
    redirect_uri: String,
    status: String,
    auth_url: String,
    error_message: Option<String>,
    expires_at: String,
    consumed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
struct OauthMailboxSessionRow {
    session_id: String,
    remote_email_id: String,
    email_address: String,
    email_domain: String,
    mailbox_source: Option<String>,
    latest_code_value: Option<String>,
    latest_code_source: Option<String>,
    latest_code_updated_at: Option<String>,
    invite_subject: Option<String>,
    invite_copy_value: Option<String>,
    invite_copy_label: Option<String>,
    invite_updated_at: Option<String>,
    invited: i64,
    last_message_id: Option<String>,
    created_at: String,
    updated_at: String,
    expires_at: String,
}

pub(crate) async fn ensure_upstream_accounts_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'codex',
            display_name TEXT NOT NULL,
            group_name TEXT,
            is_mother INTEGER NOT NULL DEFAULT 0,
            note TEXT,
            status TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            email TEXT,
            chatgpt_account_id TEXT,
            chatgpt_user_id TEXT,
            plan_type TEXT,
            plan_type_observed_at TEXT,
            masked_api_key TEXT,
            encrypted_credentials TEXT,
            token_expires_at TEXT,
            last_refreshed_at TEXT,
            last_synced_at TEXT,
            last_successful_sync_at TEXT,
            last_error TEXT,
            last_error_at TEXT,
            last_action TEXT,
            last_action_source TEXT,
            last_action_reason_code TEXT,
            last_action_reason_message TEXT,
            last_action_http_status INTEGER,
            last_action_invoke_id TEXT,
            last_action_at TEXT,
            last_activity_at TEXT,
            last_selected_at TEXT,
            last_route_failure_at TEXT,
            last_route_failure_kind TEXT,
            cooldown_until TEXT,
            consecutive_route_failures INTEGER NOT NULL DEFAULT 0,
            temporary_route_failure_streak_started_at TEXT,
            compact_support_status TEXT,
            compact_support_observed_at TEXT,
            compact_support_reason TEXT,
            local_primary_limit REAL,
            local_secondary_limit REAL,
            local_limit_unit TEXT,
            upstream_base_url TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_accounts table existence")?;

    ensure_nullable_text_column(pool, "pool_upstream_accounts", "group_name")
        .await
        .context("failed to ensure pool_upstream_accounts.group_name")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_selected_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_selected_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_route_failure_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_route_failure_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_route_failure_kind")
        .await
        .context("failed to ensure pool_upstream_accounts.last_route_failure_kind")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "cooldown_until")
        .await
        .context("failed to ensure pool_upstream_accounts.cooldown_until")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "compact_support_status")
        .await
        .context("failed to ensure pool_upstream_accounts.compact_support_status")?;
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "compact_support_observed_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.compact_support_observed_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "compact_support_reason")
        .await
        .context("failed to ensure pool_upstream_accounts.compact_support_reason")?;
    ensure_integer_column_with_default(pool, "pool_upstream_accounts", "is_mother", "0")
        .await
        .context("failed to ensure pool_upstream_accounts.is_mother")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "upstream_base_url")
        .await
        .context("failed to ensure pool_upstream_accounts.upstream_base_url")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "plan_type_observed_at")
        .await
        .context("failed to ensure pool_upstream_accounts.plan_type_observed_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_activity_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_activity_at")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_source")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_source")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_reason_code")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_reason_code")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_reason_message")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_reason_message")?;
    ensure_nullable_integer_column(pool, "pool_upstream_accounts", "last_action_http_status")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_http_status")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_invoke_id")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_invoke_id")?;
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "last_action_at")
        .await
        .context("failed to ensure pool_upstream_accounts.last_action_at")?;
    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE pool_upstream_accounts
        ADD COLUMN last_activity_live_backfill_completed INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context(
            "failed to ensure pool_upstream_accounts.last_activity_live_backfill_completed",
        );
    }
    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE pool_upstream_accounts
        ADD COLUMN last_activity_archive_backfill_completed INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context(
            "failed to ensure pool_upstream_accounts.last_activity_archive_backfill_completed",
        );
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE pool_upstream_accounts
        ADD COLUMN consecutive_route_failures INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err)
            .context("failed to ensure pool_upstream_accounts.consecutive_route_failures");
    }
    ensure_nullable_text_column(
        pool,
        "pool_upstream_accounts",
        "temporary_route_failure_streak_started_at",
    )
    .await
    .context("failed to ensure pool_upstream_accounts.temporary_route_failure_streak_started_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_kind_enabled
        ON pool_upstream_accounts (kind, enabled)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_kind_enabled")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_chatgpt_account_id
        ON pool_upstream_accounts (chatgpt_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_chatgpt_account_id")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            action TEXT NOT NULL,
            source TEXT NOT NULL,
            reason_code TEXT,
            reason_message TEXT,
            http_status INTEGER,
            failure_kind TEXT,
            invoke_id TEXT,
            sticky_key TEXT,
            created_at TEXT NOT NULL,
            FOREIGN KEY(account_id) REFERENCES pool_upstream_accounts(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_events table existence")?;
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_events_account_time
        ON pool_upstream_account_events (account_id, occurred_at DESC, id DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_events_account_time")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_oauth_login_sessions (
            login_id TEXT PRIMARY KEY,
            account_id INTEGER,
            display_name TEXT,
            group_name TEXT,
            group_bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            is_mother INTEGER NOT NULL DEFAULT 0,
            note TEXT,
            tag_ids_json TEXT,
            group_note TEXT,
            mailbox_session_id TEXT,
            generated_mailbox_address TEXT,
            state TEXT NOT NULL UNIQUE,
            pkce_verifier TEXT NOT NULL,
            redirect_uri TEXT NOT NULL,
            status TEXT NOT NULL,
            auth_url TEXT NOT NULL,
            error_message TEXT,
            expires_at TEXT NOT NULL,
            consumed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_oauth_login_sessions table existence")?;

    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "group_name")
        .await
        .context("failed to ensure pool_oauth_login_sessions.group_name")?;
    let existing_oauth_login_session_columns =
        load_sqlite_table_columns(pool, "pool_oauth_login_sessions").await?;
    if !existing_oauth_login_session_columns.contains("group_bound_proxy_keys_json") {
        sqlx::query(
            r#"
            ALTER TABLE pool_oauth_login_sessions
            ADD COLUMN group_bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]'
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_oauth_login_sessions.group_bound_proxy_keys_json")?;
    }
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "group_note")
        .await
        .context("failed to ensure pool_oauth_login_sessions.group_note")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "mailbox_session_id")
        .await
        .context("failed to ensure pool_oauth_login_sessions.mailbox_session_id")?;
    ensure_nullable_text_column(
        pool,
        "pool_oauth_login_sessions",
        "generated_mailbox_address",
    )
    .await
    .context("failed to ensure pool_oauth_login_sessions.generated_mailbox_address")?;
    ensure_integer_column_with_default(pool, "pool_oauth_login_sessions", "is_mother", "0")
        .await
        .context("failed to ensure pool_oauth_login_sessions.is_mother")?;
    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "tag_ids_json")
        .await
        .context("failed to ensure pool_oauth_login_sessions.tag_ids_json")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_tags (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            guard_enabled INTEGER NOT NULL DEFAULT 0,
            lookback_hours INTEGER,
            max_conversations INTEGER,
            allow_cut_out INTEGER NOT NULL DEFAULT 1,
            allow_cut_in INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_tags table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_tags (
            account_id INTEGER NOT NULL,
            tag_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (account_id, tag_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_tags table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_account_tags_tag_id
        ON pool_upstream_account_tags (tag_id, updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_account_tags_tag_id")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_oauth_mailbox_sessions (
            session_id TEXT PRIMARY KEY,
            remote_email_id TEXT NOT NULL,
            email_address TEXT NOT NULL,
            email_domain TEXT NOT NULL,
            mailbox_source TEXT,
            latest_code_value TEXT,
            latest_code_source TEXT,
            latest_code_updated_at TEXT,
            invite_subject TEXT,
            invite_copy_value TEXT,
            invite_copy_label TEXT,
            invite_updated_at TEXT,
            invited INTEGER NOT NULL DEFAULT 0,
            last_message_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_oauth_mailbox_sessions table existence")?;
    ensure_nullable_text_column(pool, "pool_oauth_mailbox_sessions", "mailbox_source")
        .await
        .context("failed to ensure pool_oauth_mailbox_sessions.mailbox_source")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_oauth_mailbox_sessions_expires_at
        ON pool_oauth_mailbox_sessions (expires_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_oauth_mailbox_sessions_expires_at")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_group_notes (
            group_name TEXT PRIMARY KEY,
            note TEXT NOT NULL,
            bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0,
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_group_notes table existence")?;
    let existing_group_note_columns =
        load_sqlite_table_columns(pool, "pool_upstream_account_group_notes").await?;
    if !existing_group_note_columns.contains("bound_proxy_keys_json") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]'
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.bound_proxy_keys_json")?;
    }
    if !existing_group_note_columns.contains("upstream_429_retry_enabled") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN upstream_429_retry_enabled INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.upstream_429_retry_enabled")?;
    }
    if !existing_group_note_columns.contains("upstream_429_max_retries") {
        sqlx::query(
            r#"
            ALTER TABLE pool_upstream_account_group_notes
            ADD COLUMN upstream_429_max_retries INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add pool_upstream_account_group_notes.upstream_429_max_retries")?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_limit_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            captured_at TEXT NOT NULL,
            limit_id TEXT,
            limit_name TEXT,
            plan_type TEXT,
            primary_used_percent REAL,
            primary_window_minutes INTEGER,
            primary_resets_at TEXT,
            secondary_used_percent REAL,
            secondary_window_minutes INTEGER,
            secondary_resets_at TEXT,
            credits_has_credits INTEGER,
            credits_unlimited INTEGER,
            credits_balance TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_limit_samples table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_limit_samples_account_captured_at
        ON pool_upstream_account_limit_samples (account_id, captured_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_limit_samples_account_captured_at")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_sticky_routes (
            sticky_key TEXT PRIMARY KEY,
            account_id INTEGER NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_sticky_routes table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_sticky_routes_account_updated
        ON pool_sticky_routes (account_id, updated_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_sticky_routes_account_updated")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_sticky_routes_account_last_seen
        ON pool_sticky_routes (account_id, last_seen_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_sticky_routes_account_last_seen")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_routing_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            encrypted_api_key TEXT,
            masked_api_key TEXT,
            primary_sync_interval_secs INTEGER,
            secondary_sync_interval_secs INTEGER,
            priority_available_account_cap INTEGER,
            responses_first_byte_timeout_secs INTEGER,
            compact_first_byte_timeout_secs INTEGER,
            responses_stream_timeout_secs INTEGER,
            compact_stream_timeout_secs INTEGER,
            default_first_byte_timeout_secs INTEGER,
            upstream_handshake_timeout_secs INTEGER,
            request_read_timeout_secs INTEGER,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_routing_settings table existence")?;
    ensure_nullable_integer_column(pool, "pool_routing_settings", "primary_sync_interval_secs")
        .await
        .context("failed to ensure pool_routing_settings.primary_sync_interval_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "secondary_sync_interval_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.secondary_sync_interval_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "priority_available_account_cap",
    )
    .await
    .context("failed to ensure pool_routing_settings.priority_available_account_cap")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "responses_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.responses_first_byte_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "compact_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.compact_first_byte_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "responses_stream_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.responses_stream_timeout_secs")?;
    ensure_nullable_integer_column(pool, "pool_routing_settings", "compact_stream_timeout_secs")
        .await
        .context("failed to ensure pool_routing_settings.compact_stream_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "default_first_byte_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.default_first_byte_timeout_secs")?;
    ensure_nullable_integer_column(
        pool,
        "pool_routing_settings",
        "upstream_handshake_timeout_secs",
    )
    .await
    .context("failed to ensure pool_routing_settings.upstream_handshake_timeout_secs")?;
    ensure_nullable_integer_column(pool, "pool_routing_settings", "request_read_timeout_secs")
        .await
        .context("failed to ensure pool_routing_settings.request_read_timeout_secs")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pool_routing_settings (
            id,
            encrypted_api_key,
            masked_api_key,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            default_first_byte_timeout_secs,
            upstream_handshake_timeout_secs,
            request_read_timeout_secs
        ) VALUES (?1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to ensure default pool_routing_settings row")?;

    Ok(())
}

pub(crate) fn spawn_upstream_account_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("upstream account maintenance stopped");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = run_upstream_account_maintenance_once(state.clone()).await {
                        warn!(error = %err, "failed to run upstream account maintenance");
                    }
                }
            }
        }
    })
}

async fn ensure_nullable_text_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} TEXT");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

async fn ensure_nullable_integer_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} INTEGER");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

async fn sqlite_table_exists(pool: &Pool<Sqlite>, table_name: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await?
        > 0)
}

#[cfg(test)]
pub(crate) async fn list_upstream_accounts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListUpstreamAccountsQuery>,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    list_upstream_accounts_from_params(state, params).await
}

pub(crate) async fn list_upstream_accounts_from_uri(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    let params = parse_list_upstream_accounts_query(&original_uri)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    list_upstream_accounts_from_params(state, params).await
}

async fn list_upstream_accounts_from_params(
    state: Arc<AppState>,
    params: ListUpstreamAccountsQuery,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let page = normalize_upstream_account_list_page(params.page);
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let all_items = load_upstream_account_summaries_filtered(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    let total = all_items.len();
    let metrics = build_upstream_account_list_metrics(&all_items);
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let mut items = if offset >= total {
        Vec::new()
    } else {
        all_items
            .into_iter()
            .skip(offset)
            .take(page_size)
            .collect::<Vec<_>>()
    };
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
        .await
        .map_err(internal_error_tuple)?;
    let groups = load_upstream_account_groups(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let bound_proxy_keys = groups
        .iter()
        .flat_map(|group| group.bound_proxy_keys.iter().cloned())
        .collect::<Vec<_>>();
    let forward_proxy_nodes =
        build_forward_proxy_binding_nodes_response(state.as_ref(), &bound_proxy_keys)
            .await
            .map_err(internal_error_tuple)?;
    let has_ungrouped_accounts = has_ungrouped_upstream_accounts(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let routing = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        items,
        total,
        page,
        page_size,
        metrics,
        groups,
        forward_proxy_nodes,
        has_ungrouped_accounts,
        routing: build_pool_routing_settings_response(state.as_ref(), &routing),
    }))
}

fn parse_list_upstream_accounts_query(uri: &Uri) -> Result<ListUpstreamAccountsQuery, String> {
    let base = Query::<ListUpstreamAccountsBaseQuery>::try_from_uri(uri)
        .map_err(|err| err.body_text())?
        .0;
    let mut params = ListUpstreamAccountsQuery {
        group_search: base.group_search,
        group_ungrouped: base.group_ungrouped,
        status: base.status,
        page: base.page,
        page_size: base.page_size,
        tag_ids: base.tag_ids,
        ..ListUpstreamAccountsQuery::default()
    };

    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "workStatus" => params.work_status.push(value.into_owned()),
            "enableStatus" => params.enable_status.push(value.into_owned()),
            "healthStatus" => params.health_status.push(value.into_owned()),
            _ => {}
        }
    }

    Ok(params)
}

pub(crate) async fn list_tags(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListTagsQuery>,
) -> Result<Json<TagListResponse>, (StatusCode, String)> {
    let items = load_tag_summaries(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(TagListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        items,
    }))
}

pub(crate) async fn create_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateTagRequest>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let name = normalize_tag_name(&payload.name)?;
    let rule = normalize_tag_rule(
        payload.guard_enabled,
        payload.lookback_hours,
        payload.max_conversations,
        payload.allow_cut_out,
        payload.allow_cut_in,
    )?;
    let detail = insert_tag(&state.pool, &name, &rule)
        .await
        .map_err(map_tag_write_error)?;
    Ok(Json(detail))
}

pub(crate) async fn get_tag(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    let detail = load_tag_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    Ok(Json(detail))
}

pub(crate) async fn update_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateTagRequest>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let existing = load_tag_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    let name = match payload.name {
        Some(value) => normalize_tag_name(&value)?,
        None => existing.name.clone(),
    };
    let rule = normalize_tag_rule(
        payload.guard_enabled.unwrap_or(existing.guard_enabled != 0),
        payload.lookback_hours.or(existing.lookback_hours),
        payload.max_conversations.or(existing.max_conversations),
        payload.allow_cut_out.unwrap_or(existing.allow_cut_out != 0),
        payload.allow_cut_in.unwrap_or(existing.allow_cut_in != 0),
    )?;
    let detail = persist_tag_update(&state.pool, id, &name, &rule)
        .await
        .map_err(map_tag_write_error)?;
    Ok(Json(detail))
}

pub(crate) async fn delete_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    delete_tag_by_id(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn update_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
    Json(payload): Json<UpdateUpstreamAccountGroupRequest>,
) -> Result<Json<UpstreamAccountGroupSummary>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    let group_name = normalize_optional_text(Some(group_name)).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "group name is required".to_string(),
        )
    })?;
    let note = normalize_optional_text(payload.note);
    let bound_proxy_keys_was_updated = payload.bound_proxy_keys.is_some();
    let bound_proxy_keys = payload
        .bound_proxy_keys
        .map(normalize_bound_proxy_keys)
        .unwrap_or_else(Vec::new);
    let upstream_429_retry_enabled_was_updated = payload.upstream_429_retry_enabled.is_some();
    let upstream_429_max_retries_was_updated = payload.upstream_429_max_retries.is_some();
    let normalized_upstream_429_max_retries = payload
        .upstream_429_max_retries
        .map(normalize_group_upstream_429_max_retries)
        .unwrap_or_default();

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    if !group_has_accounts_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?
    {
        return Err((StatusCode::NOT_FOUND, "group not found".to_string()));
    }
    let existing_metadata = load_group_metadata_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?
        .unwrap_or_default();
    if bound_proxy_keys_was_updated && !bound_proxy_keys.is_empty() {
        let has_selectable_bound_proxy_keys = {
            let manager = state.forward_proxy.lock().await;
            manager.has_selectable_bound_proxy_keys(&bound_proxy_keys)
        };
        if !has_selectable_bound_proxy_keys {
            return Err((
                StatusCode::BAD_REQUEST,
                "select at least one available proxy node or clear bindings before saving"
                    .to_string(),
            ));
        }
    }
    save_group_metadata_record_conn(
        tx.as_mut(),
        &group_name,
        UpstreamAccountGroupMetadata {
            note,
            bound_proxy_keys: if bound_proxy_keys_was_updated {
                bound_proxy_keys
            } else {
                existing_metadata.bound_proxy_keys
            },
            upstream_429_retry_enabled: if upstream_429_retry_enabled_was_updated {
                payload.upstream_429_retry_enabled.unwrap_or(false)
            } else {
                existing_metadata.upstream_429_retry_enabled
            },
            upstream_429_max_retries: if upstream_429_max_retries_was_updated {
                normalized_upstream_429_max_retries
            } else {
                existing_metadata.upstream_429_max_retries
            },
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;

    let saved = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountGroupSummary {
        group_name,
        note: saved.note,
        bound_proxy_keys: saved.bound_proxy_keys,
        upstream_429_retry_enabled: saved.upstream_429_retry_enabled,
        upstream_429_max_retries: saved.upstream_429_max_retries,
    }))
}

pub(crate) async fn get_upstream_account(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}

async fn build_imported_oauth_validation_response(
    state: &AppState,
    items: &[ImportOauthCredentialFileRequest],
    binding: &ResolvedRequiredGroupProxyBinding,
) -> ImportedOauthValidationResponse {
    let mut seen_keys = HashSet::new();
    let mut rows = Vec::with_capacity(items.len());

    for item in items {
        let normalized = match normalize_imported_oauth_credentials(item) {
            Ok(value) => value,
            Err(message) => {
                rows.push(ImportedOauthValidationRow {
                    source_id: item.source_id.clone(),
                    file_name: item.file_name.clone(),
                    email: None,
                    chatgpt_account_id: None,
                    display_name: None,
                    token_expires_at: None,
                    matched_account: None,
                    status: IMPORT_VALIDATION_STATUS_INVALID.to_string(),
                    detail: Some(message),
                    attempts: 0,
                });
                continue;
            }
        };

        let match_key = imported_match_key(&normalized.email, &normalized.chatgpt_account_id);
        if !seen_keys.insert(match_key) {
            rows.push(ImportedOauthValidationRow {
                source_id: normalized.source_id,
                file_name: normalized.file_name,
                email: Some(normalized.email),
                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                display_name: Some(normalized.display_name),
                token_expires_at: Some(normalized.token_expires_at),
                matched_account: None,
                status: IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT.to_string(),
                detail: Some("duplicate credential in current import selection".to_string()),
                attempts: 0,
            });
            continue;
        }
        rows.push(
            build_imported_oauth_validation_result(state, normalized, binding)
                .await
                .0,
        );
    }

    build_imported_oauth_validation_response_from_rows(items.len(), rows)
}

fn build_imported_oauth_pending_response(
    items: &[ImportOauthCredentialFileRequest],
) -> ImportedOauthValidationResponse {
    ImportedOauthValidationResponse {
        input_files: items.len(),
        unique_in_input: items.len(),
        duplicate_in_input: 0,
        rows: items
            .iter()
            .map(|item| ImportedOauthValidationRow {
                source_id: item.source_id.clone(),
                file_name: item.file_name.clone(),
                email: None,
                chatgpt_account_id: None,
                display_name: None,
                token_expires_at: None,
                matched_account: None,
                status: "pending".to_string(),
                detail: None,
                attempts: 0,
            })
            .collect(),
    }
}

fn build_imported_oauth_validation_response_from_rows(
    input_files: usize,
    rows: Vec<ImportedOauthValidationRow>,
) -> ImportedOauthValidationResponse {
    let duplicate_in_input = rows
        .iter()
        .filter(|row| row.status == IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT)
        .count();
    ImportedOauthValidationResponse {
        input_files,
        unique_in_input: rows.len().saturating_sub(duplicate_in_input),
        duplicate_in_input,
        rows,
    }
}

fn compute_imported_oauth_validation_counts(
    rows: &[ImportedOauthValidationRow],
) -> ImportedOauthValidationCounts {
    let mut counts = ImportedOauthValidationCounts::default();
    for row in rows {
        match row.status.as_str() {
            IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT => counts.duplicate_in_input += 1,
            IMPORT_VALIDATION_STATUS_OK => counts.ok += 1,
            IMPORT_VALIDATION_STATUS_OK_EXHAUSTED => counts.ok_exhausted += 1,
            IMPORT_VALIDATION_STATUS_INVALID => counts.invalid += 1,
            IMPORT_VALIDATION_STATUS_ERROR => counts.error += 1,
            "pending" => counts.pending += 1,
            _ => counts.error += 1,
        }
    }
    counts.checked =
        counts.duplicate_in_input + counts.ok + counts.ok_exhausted + counts.invalid + counts.error;
    counts
}

fn build_imported_oauth_snapshot_event(
    snapshot: ImportedOauthValidationResponse,
) -> ImportedOauthValidationSnapshotEvent {
    let counts = compute_imported_oauth_validation_counts(&snapshot.rows);
    ImportedOauthValidationSnapshotEvent { snapshot, counts }
}

fn compute_bulk_upstream_account_sync_counts(
    rows: &[BulkUpstreamAccountSyncRow],
) -> BulkUpstreamAccountSyncCounts {
    let mut counts = BulkUpstreamAccountSyncCounts {
        total: rows.len(),
        completed: 0,
        succeeded: 0,
        failed: 0,
        skipped: 0,
    };
    for row in rows {
        match row.status.as_str() {
            BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED => {
                counts.succeeded += 1;
                counts.completed += 1;
            }
            BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED => {
                counts.failed += 1;
                counts.completed += 1;
            }
            BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED => {
                counts.skipped += 1;
                counts.completed += 1;
            }
            _ => {}
        }
    }
    counts
}

fn with_bulk_upstream_account_sync_snapshot_status(
    mut snapshot: BulkUpstreamAccountSyncSnapshot,
    status: &str,
) -> BulkUpstreamAccountSyncSnapshot {
    snapshot.status = status.to_string();
    snapshot
}

fn build_bulk_upstream_account_sync_snapshot_event(
    snapshot: BulkUpstreamAccountSyncSnapshot,
) -> BulkUpstreamAccountSyncSnapshotEvent {
    let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
    BulkUpstreamAccountSyncSnapshotEvent { snapshot, counts }
}

fn imported_oauth_sse_event<T: Serialize>(event_name: &str, payload: &T) -> Option<Event> {
    match Event::default().event(event_name).json_data(payload) {
        Ok(event) => Some(event),
        Err(err) => {
            warn!(
                ?err,
                event_name, "failed to serialize imported oauth validation event"
            );
            None
        }
    }
}

fn bulk_upstream_account_sync_sse_event<T: Serialize>(
    event_name: &str,
    payload: &T,
) -> Option<Event> {
    match Event::default().event(event_name).json_data(payload) {
        Ok(event) => Some(event),
        Err(err) => {
            warn!(
                ?err,
                event_name, "failed to serialize bulk upstream account sync event"
            );
            None
        }
    }
}

fn imported_oauth_terminal_event_to_sse(
    terminal: &ImportedOauthValidationTerminalEvent,
) -> Option<Event> {
    match terminal {
        ImportedOauthValidationTerminalEvent::Completed(payload) => {
            imported_oauth_sse_event("completed", payload)
        }
        ImportedOauthValidationTerminalEvent::Failed(payload) => {
            imported_oauth_sse_event("failed", payload)
        }
        ImportedOauthValidationTerminalEvent::Cancelled(payload) => {
            imported_oauth_sse_event("cancelled", payload)
        }
    }
}

fn bulk_upstream_account_sync_terminal_event_to_sse(
    terminal: &BulkUpstreamAccountSyncTerminalEvent,
) -> Option<Event> {
    match terminal {
        BulkUpstreamAccountSyncTerminalEvent::Completed(payload) => {
            bulk_upstream_account_sync_sse_event("completed", payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Failed(payload) => {
            bulk_upstream_account_sync_sse_event("failed", payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(payload) => {
            bulk_upstream_account_sync_sse_event("cancelled", payload)
        }
    }
}

async fn build_imported_oauth_validation_result(
    state: &AppState,
    normalized: NormalizedImportedOauthCredentials,
    binding: &ResolvedRequiredGroupProxyBinding,
) -> (
    ImportedOauthValidationRow,
    Option<ImportedOauthValidatedImportData>,
) {
    let matched_account = match find_existing_import_match(
        &state.pool,
        &normalized.chatgpt_account_id,
        &normalized.email,
    )
    .await
    {
        Ok(value) => value.map(|row| import_match_summary_from_row(&row)),
        Err(err) => {
            return (
                ImportedOauthValidationRow {
                    source_id: normalized.source_id,
                    file_name: normalized.file_name,
                    email: Some(normalized.email),
                    chatgpt_account_id: Some(normalized.chatgpt_account_id),
                    display_name: Some(normalized.display_name),
                    token_expires_at: Some(normalized.token_expires_at),
                    matched_account: None,
                    status: IMPORT_VALIDATION_STATUS_ERROR.to_string(),
                    detail: Some(err.to_string()),
                    attempts: 0,
                },
                None,
            );
        }
    };

    match probe_imported_oauth_credentials(state, &normalized, binding).await {
        Ok(outcome) => (
            ImportedOauthValidationRow {
                source_id: normalized.source_id.clone(),
                file_name: normalized.file_name.clone(),
                email: Some(normalized.email.clone()),
                chatgpt_account_id: Some(normalized.chatgpt_account_id.clone()),
                display_name: Some(normalized.display_name.clone()),
                token_expires_at: Some(outcome.token_expires_at.clone()),
                matched_account,
                status: if outcome.exhausted {
                    IMPORT_VALIDATION_STATUS_OK_EXHAUSTED.to_string()
                } else {
                    IMPORT_VALIDATION_STATUS_OK.to_string()
                },
                detail: if outcome.exhausted {
                    Some("usage snapshot indicates the account is currently exhausted".to_string())
                } else {
                    outcome.usage_snapshot_warning.clone()
                },
                attempts: 1,
            },
            Some(ImportedOauthValidatedImportData {
                normalized,
                probe: outcome,
            }),
        ),
        Err(err) => (
            ImportedOauthValidationRow {
                source_id: normalized.source_id,
                file_name: normalized.file_name,
                email: Some(normalized.email),
                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                display_name: Some(normalized.display_name),
                token_expires_at: Some(normalized.token_expires_at),
                matched_account,
                status: if is_import_invalid_error_message(&err.to_string()) {
                    IMPORT_VALIDATION_STATUS_INVALID.to_string()
                } else {
                    IMPORT_VALIDATION_STATUS_ERROR.to_string()
                },
                detail: Some(err.to_string()),
                attempts: 1,
            },
            None,
        ),
    }
}

async fn update_imported_oauth_validation_job_row(
    job: &Arc<ImportedOauthValidationJob>,
    row_index: usize,
    row: ImportedOauthValidationRow,
    validated_import: Option<ImportedOauthValidatedImportData>,
) {
    let counts = {
        let mut snapshot = job.snapshot.lock().await;
        if let Some(target) = snapshot.rows.get_mut(row_index) {
            *target = row.clone();
        } else {
            return;
        }
        snapshot.duplicate_in_input = snapshot
            .rows
            .iter()
            .filter(|candidate| candidate.status == IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT)
            .count();
        snapshot.unique_in_input = snapshot
            .rows
            .len()
            .saturating_sub(snapshot.duplicate_in_input);
        compute_imported_oauth_validation_counts(&snapshot.rows)
    };
    let source_id = row.source_id.clone();
    let mut validated_imports = job.validated_imports.lock().await;
    if let Some(validated_import) = validated_import {
        validated_imports.insert(source_id, validated_import);
    } else {
        validated_imports.remove(&source_id);
    }
    let _ = job.broadcaster.send(ImportedOauthValidationJobEvent::Row(
        ImportedOauthValidationRowEvent { row, counts },
    ));
}

async fn set_imported_oauth_validation_job_terminal(
    job: &Arc<ImportedOauthValidationJob>,
    terminal: ImportedOauthValidationTerminalEvent,
) {
    {
        let mut guard = job.terminal_event.lock().await;
        if guard.is_some() {
            return;
        }
        *guard = Some(terminal.clone());
    }
    let _ = job.broadcaster.send(match terminal {
        ImportedOauthValidationTerminalEvent::Completed(payload) => {
            ImportedOauthValidationJobEvent::Completed(payload)
        }
        ImportedOauthValidationTerminalEvent::Failed(payload) => {
            ImportedOauthValidationJobEvent::Failed(payload)
        }
        ImportedOauthValidationTerminalEvent::Cancelled(payload) => {
            ImportedOauthValidationJobEvent::Cancelled(payload)
        }
    });
}

async fn finish_imported_oauth_validation_job_completed(job: &Arc<ImportedOauthValidationJob>) {
    let snapshot = { job.snapshot.lock().await.clone() };
    set_imported_oauth_validation_job_terminal(
        job,
        ImportedOauthValidationTerminalEvent::Completed(build_imported_oauth_snapshot_event(
            snapshot,
        )),
    )
    .await;
}

async fn finish_imported_oauth_validation_job_failed(
    job: &Arc<ImportedOauthValidationJob>,
    error: String,
) {
    let snapshot = { job.snapshot.lock().await.clone() };
    set_imported_oauth_validation_job_terminal(
        job,
        ImportedOauthValidationTerminalEvent::Failed(ImportedOauthValidationFailedEvent {
            counts: compute_imported_oauth_validation_counts(&snapshot.rows),
            snapshot,
            error,
        }),
    )
    .await;
}

async fn finish_imported_oauth_validation_job_cancelled(job: &Arc<ImportedOauthValidationJob>) {
    let snapshot = { job.snapshot.lock().await.clone() };
    set_imported_oauth_validation_job_terminal(
        job,
        ImportedOauthValidationTerminalEvent::Cancelled(build_imported_oauth_snapshot_event(
            snapshot,
        )),
    )
    .await;
}

async fn update_bulk_upstream_account_sync_job_row(
    job: &Arc<BulkUpstreamAccountSyncJob>,
    row: BulkUpstreamAccountSyncRow,
) {
    let counts = {
        let mut snapshot = job.snapshot.lock().await;
        if let Some(target) = snapshot
            .rows
            .iter_mut()
            .find(|candidate| candidate.account_id == row.account_id)
        {
            *target = row.clone();
        } else {
            return;
        }
        compute_bulk_upstream_account_sync_counts(&snapshot.rows)
    };
    let _ = job.broadcaster.send(BulkUpstreamAccountSyncJobEvent::Row(
        BulkUpstreamAccountSyncRowEvent { row, counts },
    ));
}

async fn set_bulk_upstream_account_sync_job_terminal(
    job: &Arc<BulkUpstreamAccountSyncJob>,
    terminal: BulkUpstreamAccountSyncTerminalEvent,
) {
    let next_status = match &terminal {
        BulkUpstreamAccountSyncTerminalEvent::Completed(_) => {
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
        }
        BulkUpstreamAccountSyncTerminalEvent::Failed(_) => {
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
        }
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(_) => {
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
        }
    };
    {
        let mut guard = job.terminal_event.lock().await;
        if guard.is_some() {
            return;
        }
        *guard = Some(terminal.clone());
    }
    {
        let mut snapshot = job.snapshot.lock().await;
        snapshot.status = next_status.to_string();
    }
    let _ = job.broadcaster.send(match terminal {
        BulkUpstreamAccountSyncTerminalEvent::Completed(payload) => {
            BulkUpstreamAccountSyncJobEvent::Completed(payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Failed(payload) => {
            BulkUpstreamAccountSyncJobEvent::Failed(payload)
        }
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(payload) => {
            BulkUpstreamAccountSyncJobEvent::Cancelled(payload)
        }
    });
}

async fn finish_bulk_upstream_account_sync_job_completed(job: &Arc<BulkUpstreamAccountSyncJob>) {
    let snapshot = with_bulk_upstream_account_sync_snapshot_status(
        job.snapshot.lock().await.clone(),
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED,
    );
    set_bulk_upstream_account_sync_job_terminal(
        job,
        BulkUpstreamAccountSyncTerminalEvent::Completed(
            build_bulk_upstream_account_sync_snapshot_event(snapshot),
        ),
    )
    .await;
}

async fn finish_bulk_upstream_account_sync_job_failed(
    job: &Arc<BulkUpstreamAccountSyncJob>,
    error: String,
) {
    let snapshot = with_bulk_upstream_account_sync_snapshot_status(
        job.snapshot.lock().await.clone(),
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED,
    );
    set_bulk_upstream_account_sync_job_terminal(
        job,
        BulkUpstreamAccountSyncTerminalEvent::Failed(BulkUpstreamAccountSyncFailedEvent {
            counts: compute_bulk_upstream_account_sync_counts(&snapshot.rows),
            snapshot,
            error,
        }),
    )
    .await;
}

async fn finish_bulk_upstream_account_sync_job_cancelled(job: &Arc<BulkUpstreamAccountSyncJob>) {
    let snapshot = with_bulk_upstream_account_sync_snapshot_status(
        job.snapshot.lock().await.clone(),
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED,
    );
    set_bulk_upstream_account_sync_job_terminal(
        job,
        BulkUpstreamAccountSyncTerminalEvent::Cancelled(
            build_bulk_upstream_account_sync_snapshot_event(snapshot),
        ),
    )
    .await;
}

fn schedule_imported_oauth_validation_job_cleanup(
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(15 * 60)).await;
        let should_remove = match runtime.get_validation_job(&job_id).await {
            Some(job) => job.terminal_event.lock().await.is_some(),
            None => false,
        };
        if should_remove {
            runtime.remove_validation_job(&job_id).await;
        }
    });
}

fn schedule_bulk_upstream_account_sync_job_cleanup(
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
) {
    tokio::spawn(async move {
        sleep(Duration::from_secs(15 * 60)).await;
        let should_remove = match runtime.get_bulk_sync_job(&job_id).await {
            Some(job) => job.terminal_event.lock().await.is_some(),
            None => false,
        };
        if should_remove {
            runtime.remove_bulk_sync_job(&job_id).await;
        }
    });
}

fn spawn_imported_oauth_validation_job(
    state: Arc<AppState>,
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
    items: Vec<ImportOauthCredentialFileRequest>,
    binding: ResolvedRequiredGroupProxyBinding,
    job: Arc<ImportedOauthValidationJob>,
) {
    tokio::spawn(async move {
        let run_result: Result<(), String> = async {
            let mut prepared = Vec::new();
            let mut seen_keys = HashSet::new();

            for (row_index, item) in items.iter().enumerate() {
                if job.cancel.is_cancelled() {
                    finish_imported_oauth_validation_job_cancelled(&job).await;
                    return Ok(());
                }

                let normalized = match normalize_imported_oauth_credentials(item) {
                    Ok(value) => value,
                    Err(message) => {
                        update_imported_oauth_validation_job_row(
                            &job,
                            row_index,
                            ImportedOauthValidationRow {
                                source_id: item.source_id.clone(),
                                file_name: item.file_name.clone(),
                                email: None,
                                chatgpt_account_id: None,
                                display_name: None,
                                token_expires_at: None,
                                matched_account: None,
                                status: IMPORT_VALIDATION_STATUS_INVALID.to_string(),
                                detail: Some(message),
                                attempts: 0,
                            },
                            None,
                        )
                        .await;
                        continue;
                    }
                };

                let match_key =
                    imported_match_key(&normalized.email, &normalized.chatgpt_account_id);
                if !seen_keys.insert(match_key) {
                    update_imported_oauth_validation_job_row(
                        &job,
                        row_index,
                        ImportedOauthValidationRow {
                            source_id: normalized.source_id,
                            file_name: normalized.file_name,
                            email: Some(normalized.email),
                            chatgpt_account_id: Some(normalized.chatgpt_account_id),
                            display_name: Some(normalized.display_name),
                            token_expires_at: Some(normalized.token_expires_at),
                            matched_account: None,
                            status: IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT.to_string(),
                            detail: Some(
                                "duplicate credential in current import selection".to_string(),
                            ),
                            attempts: 0,
                        },
                        None,
                    )
                    .await;
                    continue;
                }

                prepared.push((row_index, normalized));
            }

            let validations = stream::iter(prepared.into_iter().map(|(row_index, normalized)| {
                let state = state.clone();
                let binding = binding.clone();
                async move {
                    (
                        row_index,
                        build_imported_oauth_validation_result(
                            state.as_ref(),
                            normalized,
                            &binding,
                        )
                        .await,
                    )
                }
            }))
            .buffer_unordered(4);
            tokio::pin!(validations);

            loop {
                tokio::select! {
                    _ = job.cancel.cancelled() => {
                        finish_imported_oauth_validation_job_cancelled(&job).await;
                        return Ok(());
                    }
                    next = validations.next() => {
                        match next {
                            Some((row_index, (row, validated_import))) => {
                                update_imported_oauth_validation_job_row(
                                    &job,
                                    row_index,
                                    row,
                                    validated_import,
                                )
                                .await;
                            }
                            None => break,
                        }
                    }
                }
            }

            if job.cancel.is_cancelled() {
                finish_imported_oauth_validation_job_cancelled(&job).await;
                return Ok(());
            }

            finish_imported_oauth_validation_job_completed(&job).await;
            Ok(())
        }
        .await;

        if let Err(error) = run_result {
            finish_imported_oauth_validation_job_failed(&job, error).await;
        }

        schedule_imported_oauth_validation_job_cleanup(runtime, job_id);
    });
}

fn spawn_bulk_upstream_account_sync_job(
    state: Arc<AppState>,
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
    account_ids: Vec<i64>,
    job: Arc<BulkUpstreamAccountSyncJob>,
) {
    tokio::spawn(async move {
        let run_result: Result<(), String> = async {
            for account_id in account_ids {
                if job.cancel.is_cancelled() {
                    finish_bulk_upstream_account_sync_job_cancelled(&job).await;
                    return Ok(());
                }

                let maybe_row = load_upstream_account_row(&state.pool, account_id)
                    .await
                    .map_err(|err| err.to_string())?;
                let Some(row) = maybe_row else {
                    update_bulk_upstream_account_sync_job_row(
                        &job,
                        BulkUpstreamAccountSyncRow {
                            account_id,
                            display_name: format!("Account {account_id}"),
                            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED.to_string(),
                            detail: Some("account not found".to_string()),
                        },
                    )
                    .await;
                    continue;
                };

                if row.enabled == 0 {
                    update_bulk_upstream_account_sync_job_row(
                        &job,
                        BulkUpstreamAccountSyncRow {
                            account_id,
                            display_name: row.display_name.clone(),
                            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED.to_string(),
                            detail: Some("disabled accounts cannot be synced".to_string()),
                        },
                    )
                    .await;
                    continue;
                }

                let sync_result = state
                    .upstream_accounts
                    .account_ops
                    .run_manual_sync(state.clone(), account_id)
                    .await;
                let (status, detail) = match sync_result {
                    Ok(detail)
                        if detail.summary.display_status == UPSTREAM_ACCOUNT_STATUS_ACTIVE =>
                    {
                        (BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED, None)
                    }
                    Ok(detail) => (
                        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED,
                        detail.summary.last_error.clone().or_else(|| {
                            Some(format!(
                                "sync finished with status {}",
                                detail.summary.display_status
                            ))
                        }),
                    ),
                    Err(err) => (
                        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED,
                        Some(err.to_string()),
                    ),
                };
                update_bulk_upstream_account_sync_job_row(
                    &job,
                    BulkUpstreamAccountSyncRow {
                        account_id,
                        display_name: row.display_name,
                        status: status.to_string(),
                        detail,
                    },
                )
                .await;
            }

            finish_bulk_upstream_account_sync_job_completed(&job).await;
            Ok(())
        }
        .await;

        if let Err(err) = run_result {
            finish_bulk_upstream_account_sync_job_failed(&job, err).await;
        }

        schedule_bulk_upstream_account_sync_job_cleanup(runtime, job_id);
    });
}

async fn build_bulk_upstream_account_sync_job_response(
    job_id: String,
    job: &Arc<BulkUpstreamAccountSyncJob>,
) -> BulkUpstreamAccountSyncJobResponse {
    let snapshot = { job.snapshot.lock().await.clone() };
    BulkUpstreamAccountSyncJobResponse {
        job_id,
        counts: compute_bulk_upstream_account_sync_counts(&snapshot.rows),
        snapshot,
    }
}

pub(crate) async fn create_imported_oauth_validation_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ValidateImportedOauthAccountsRequest>,
) -> Result<Json<ImportedOauthValidationJobResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        payload.group_name.clone(),
        payload.group_bound_proxy_keys.clone(),
    )
    .await?;
    let snapshot = build_imported_oauth_pending_response(&payload.items);
    let job_id = random_hex(16)?;
    let job = Arc::new(ImportedOauthValidationJob::new(snapshot.clone(), &binding));
    state
        .upstream_accounts
        .insert_validation_job(job_id.clone(), job.clone())
        .await;
    spawn_imported_oauth_validation_job(
        state.clone(),
        state.upstream_accounts.clone(),
        job_id.clone(),
        payload.items,
        binding,
        job,
    );
    Ok(Json(ImportedOauthValidationJobResponse {
        job_id,
        snapshot,
    }))
}

pub(crate) async fn create_bulk_upstream_account_sync_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<BulkUpstreamAccountSyncJobRequest>,
) -> Result<Json<BulkUpstreamAccountSyncJobResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let account_ids = normalize_bulk_upstream_account_ids(&payload.account_ids)?;
    let _creation_guard = state.upstream_accounts.bulk_sync_creation.lock().await;
    if let Some((job_id, job)) = state.upstream_accounts.get_running_bulk_sync_job().await {
        return Ok(Json(
            build_bulk_upstream_account_sync_job_response(job_id, &job).await,
        ));
    }
    let job_id = random_hex(16)?;
    let snapshot = BulkUpstreamAccountSyncSnapshot {
        job_id: job_id.clone(),
        status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
        rows: build_bulk_upstream_account_sync_pending_rows(&state.pool, &account_ids)
            .await
            .map_err(internal_error_tuple)?,
    };
    let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(snapshot.clone()));
    state
        .upstream_accounts
        .insert_bulk_sync_job(job_id.clone(), job.clone())
        .await;
    drop(_creation_guard);
    spawn_bulk_upstream_account_sync_job(
        state.clone(),
        state.upstream_accounts.clone(),
        job_id.clone(),
        account_ids,
        job,
    );
    Ok(Json(BulkUpstreamAccountSyncJobResponse {
        job_id,
        snapshot,
        counts,
    }))
}

pub(crate) async fn stream_imported_oauth_validation_job_events(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    let job = state
        .upstream_accounts
        .get_validation_job(&job_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "validation job not found".to_string(),
            )
        })?;
    let snapshot = { job.snapshot.lock().await.clone() };
    let terminal = { job.terminal_event.lock().await.clone() };
    let initial_events = {
        let mut events = Vec::new();
        if let Some(event) =
            imported_oauth_sse_event("snapshot", &build_imported_oauth_snapshot_event(snapshot))
        {
            events.push(Ok(event));
        }
        if let Some(terminal_event) = terminal
            .as_ref()
            .and_then(imported_oauth_terminal_event_to_sse)
        {
            events.push(Ok(terminal_event));
        }
        stream::iter(events)
    };
    let job_id_for_updates = job_id.clone();
    let updates = BroadcastStream::new(job.broadcaster.subscribe()).filter_map(move |message| {
        let lagged_job_id = job_id_for_updates.clone();
        async move {
            match message {
                Ok(ImportedOauthValidationJobEvent::Row(payload)) => {
                    imported_oauth_sse_event("row", &payload).map(Ok)
                }
                Ok(ImportedOauthValidationJobEvent::Completed(payload)) => {
                    imported_oauth_sse_event("completed", &payload).map(Ok)
                }
                Ok(ImportedOauthValidationJobEvent::Failed(payload)) => {
                    imported_oauth_sse_event("failed", &payload).map(Ok)
                }
                Ok(ImportedOauthValidationJobEvent::Cancelled(payload)) => {
                    imported_oauth_sse_event("cancelled", &payload).map(Ok)
                }
                Err(err) => {
                    warn!(
                        ?err,
                        job_id = lagged_job_id,
                        "imported oauth validation sse lagging"
                    );
                    None
                }
            }
        }
    });

    Ok(Sse::new(initial_events.chain(updates))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub(crate) async fn get_bulk_upstream_account_sync_job(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<BulkUpstreamAccountSyncJobResponse>, (StatusCode, String)> {
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&job_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "bulk sync job not found".to_string()))?;
    Ok(Json(
        build_bulk_upstream_account_sync_job_response(job_id, &job).await,
    ))
}

pub(crate) async fn stream_bulk_upstream_account_sync_job_events(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&job_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "bulk sync job not found".to_string()))?;
    let snapshot = { job.snapshot.lock().await.clone() };
    let terminal = { job.terminal_event.lock().await.clone() };
    let initial_events = {
        let mut events = Vec::new();
        if let Some(event) = bulk_upstream_account_sync_sse_event(
            "snapshot",
            &build_bulk_upstream_account_sync_snapshot_event(snapshot),
        ) {
            events.push(Ok(event));
        }
        if let Some(terminal_event) = terminal
            .as_ref()
            .and_then(bulk_upstream_account_sync_terminal_event_to_sse)
        {
            events.push(Ok(terminal_event));
        }
        stream::iter(events)
    };
    let job_id_for_updates = job_id.clone();
    let updates = BroadcastStream::new(job.broadcaster.subscribe()).filter_map(move |message| {
        let lagged_job_id = job_id_for_updates.clone();
        async move {
            match message {
                Ok(BulkUpstreamAccountSyncJobEvent::Row(payload)) => {
                    bulk_upstream_account_sync_sse_event("row", &payload).map(Ok)
                }
                Ok(BulkUpstreamAccountSyncJobEvent::Completed(payload)) => {
                    bulk_upstream_account_sync_sse_event("completed", &payload).map(Ok)
                }
                Ok(BulkUpstreamAccountSyncJobEvent::Failed(payload)) => {
                    bulk_upstream_account_sync_sse_event("failed", &payload).map(Ok)
                }
                Ok(BulkUpstreamAccountSyncJobEvent::Cancelled(payload)) => {
                    bulk_upstream_account_sync_sse_event("cancelled", &payload).map(Ok)
                }
                Err(err) => {
                    warn!(
                        ?err,
                        job_id = lagged_job_id,
                        "bulk upstream account sync sse lagging"
                    );
                    None
                }
            }
        }
    });

    Ok(Sse::new(initial_events.chain(updates))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

pub(crate) async fn cancel_imported_oauth_validation_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let Some(job) = state.upstream_accounts.get_validation_job(&job_id).await else {
        return Err((
            StatusCode::NOT_FOUND,
            "validation job not found".to_string(),
        ));
    };

    if job.terminal_event.lock().await.is_some() {
        state.upstream_accounts.remove_validation_job(&job_id).await;
        return Ok(StatusCode::NO_CONTENT);
    }

    job.cancel.cancel();
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn cancel_bulk_upstream_account_sync_job(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let Some(job) = state.upstream_accounts.get_bulk_sync_job(&job_id).await else {
        return Err((StatusCode::NOT_FOUND, "bulk sync job not found".to_string()));
    };

    if job.terminal_event.lock().await.is_some() {
        state.upstream_accounts.remove_bulk_sync_job(&job_id).await;
        return Ok(StatusCode::NO_CONTENT);
    }

    job.cancel.cancel();
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn bulk_update_upstream_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<BulkUpstreamAccountActionRequest>,
) -> Result<Json<BulkUpstreamAccountActionResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let action = normalize_bulk_upstream_account_action(&payload.action)?;
    let account_ids = normalize_bulk_upstream_account_ids(&payload.account_ids)?;
    let normalized_tag_ids = if matches!(
        action.as_str(),
        BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS | BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS
    ) {
        let tag_ids = validate_tag_ids(&state.pool, &payload.tag_ids).await?;
        if tag_ids.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "tagIds must contain at least one tag".to_string(),
            ));
        }
        tag_ids
    } else {
        Vec::new()
    };

    let mut results = Vec::with_capacity(account_ids.len());
    for account_id in &account_ids {
        let display_name = load_upstream_account_row(&state.pool, *account_id)
            .await
            .map_err(internal_error_tuple)?
            .map(|row| row.display_name);
        let outcome = apply_bulk_upstream_account_action(
            state.clone(),
            *account_id,
            action.as_str(),
            payload.group_name.clone(),
            normalized_tag_ids.clone(),
        )
        .await;
        let (status, detail) = match outcome {
            Ok(()) => ("succeeded".to_string(), None),
            Err((_, message)) => ("failed".to_string(), Some(message)),
        };
        results.push(BulkUpstreamAccountActionResult {
            account_id: *account_id,
            display_name,
            status,
            detail,
        });
    }

    let succeeded_count = results
        .iter()
        .filter(|result| result.status == "succeeded")
        .count();
    Ok(Json(BulkUpstreamAccountActionResponse {
        action,
        requested_count: account_ids.len(),
        completed_count: results.len(),
        succeeded_count,
        failed_count: results.len().saturating_sub(succeeded_count),
        results,
    }))
}

pub(crate) async fn validate_imported_oauth_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ValidateImportedOauthAccountsRequest>,
) -> Result<Json<ImportedOauthValidationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        payload.group_name,
        payload.group_bound_proxy_keys,
    )
    .await?;
    Ok(Json(
        build_imported_oauth_validation_response(state.as_ref(), &payload.items, &binding).await,
    ))
}

pub(crate) async fn import_validated_oauth_accounts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ImportValidatedOauthAccountsRequest>,
) -> Result<Json<ImportedOauthImportResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let ImportValidatedOauthAccountsRequest {
        items,
        selected_source_ids,
        validation_job_id,
        group_name,
        group_bound_proxy_keys,
        group_note,
        tag_ids,
    } = payload;
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let selected_source_ids = selected_source_ids
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .collect::<HashSet<_>>();
    if selected_source_ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "selectedSourceIds must not be empty".to_string(),
        ));
    }
    let group_name = normalize_optional_text(group_name);
    let group_note = normalize_optional_text(group_note);
    validate_group_note_target(group_name.as_deref(), group_note.is_some())?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        group_note.clone(),
        group_note.is_some(),
        group_bound_proxy_keys.clone(),
        group_bound_proxy_keys.is_some(),
    );
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        group_bound_proxy_keys.clone(),
    )
    .await?;
    let group_name = Some(resolved_group_binding.group_name.clone());
    let tag_ids = validate_tag_ids(&state.pool, &tag_ids).await?;
    let cached_validation_results = if let Some(job_id) = normalize_optional_text(validation_job_id)
    {
        if let Some(job) = state.upstream_accounts.get_validation_job(&job_id).await {
            if job.target_group_name == resolved_group_binding.group_name
                && job.target_bound_proxy_keys == resolved_group_binding.bound_proxy_keys
            {
                job.validated_imports.lock().await.clone()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };
    let input_files = items.len();
    let selected_files = selected_source_ids.len();

    let mut created = 0usize;
    let mut updated_existing = 0usize;
    let mut failed = 0usize;
    let mut seen_keys = HashSet::new();
    let mut results = Vec::new();

    for item in items {
        if !selected_source_ids.contains(&item.source_id) {
            continue;
        }

        let cached_validation = cached_validation_results.get(&item.source_id).cloned();
        let normalized = match cached_validation.as_ref() {
            Some(cached) => cached.normalized.clone(),
            None => match normalize_imported_oauth_credentials(&item) {
                Ok(value) => value,
                Err(message) => {
                    failed += 1;
                    results.push(ImportedOauthImportResult {
                        source_id: item.source_id,
                        file_name: item.file_name,
                        email: None,
                        chatgpt_account_id: None,
                        account_id: None,
                        status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                        detail: Some(message),
                        matched_account: None,
                    });
                    continue;
                }
            },
        };

        let match_key = imported_match_key(&normalized.email, &normalized.chatgpt_account_id);
        if !seen_keys.insert(match_key) {
            failed += 1;
            results.push(ImportedOauthImportResult {
                source_id: normalized.source_id,
                file_name: normalized.file_name,
                email: Some(normalized.email),
                chatgpt_account_id: Some(normalized.chatgpt_account_id),
                account_id: None,
                status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                detail: Some("duplicate credential in selected import set".to_string()),
                matched_account: None,
            });
            continue;
        }

        let existing_match = match find_existing_import_match(
            &state.pool,
            &normalized.chatgpt_account_id,
            &normalized.email,
        )
        .await
        {
            Ok(value) => value,
            Err(err) => {
                failed += 1;
                results.push(ImportedOauthImportResult {
                    source_id: normalized.source_id,
                    file_name: normalized.file_name,
                    email: Some(normalized.email),
                    chatgpt_account_id: Some(normalized.chatgpt_account_id),
                    account_id: None,
                    status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                    detail: Some(err.to_string()),
                    matched_account: None,
                });
                continue;
            }
        };
        let matched_account = existing_match.as_ref().map(import_match_summary_from_row);
        let probe = match cached_validation {
            Some(cached) => cached.probe,
            None => match probe_imported_oauth_credentials(
                state.as_ref(),
                &normalized,
                &resolved_group_binding,
            )
            .await
            {
                Ok(value) => value,
                Err(err) => {
                    failed += 1;
                    results.push(ImportedOauthImportResult {
                        source_id: normalized.source_id,
                        file_name: normalized.file_name,
                        email: Some(normalized.email),
                        chatgpt_account_id: Some(normalized.chatgpt_account_id),
                        account_id: existing_match.as_ref().map(|row| row.id),
                        status: IMPORT_RESULT_STATUS_FAILED.to_string(),
                        detail: Some(err.to_string()),
                        matched_account,
                    });
                    continue;
                }
            },
        };

        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(probe.credentials.clone()),
        )
        .map_err(internal_error_tuple)?;
        let (persisted_account_id, import_warning) = if let Some(existing_row) =
            existing_match.as_ref()
        {
            let warning = state
                .upstream_accounts
                .account_ops
                .run_persist_imported_oauth(state.clone(), existing_row.id, probe.clone())
                .await?;
            (existing_row.id, warning)
        } else {
            let persisted_account_id = {
                let mut tx = state
                    .pool
                    .begin_with("BEGIN IMMEDIATE")
                    .await
                    .map_err(internal_error_tuple)?;
                ensure_display_name_available(&mut *tx, &normalized.display_name, None).await?;
                let account_id = upsert_oauth_account(
                    &mut tx,
                    OauthAccountUpsert {
                        account_id: None,
                        display_name: &normalized.display_name,
                        group_name: group_name.clone(),
                        is_mother: false,
                        note: None,
                        tag_ids: tag_ids.clone(),
                        requested_group_metadata_changes: requested_group_metadata_changes.clone(),
                        claims: &probe.claims,
                        encrypted_credentials,
                        token_expires_at: &probe.token_expires_at,
                    },
                )
                .await
                .map_err(internal_error_tuple)?;
                tx.commit().await.map_err(internal_error_tuple)?;
                account_id
            };

            let warning = state
                .upstream_accounts
                .account_ops
                .run_persist_imported_oauth(state.clone(), persisted_account_id, probe.clone())
                .await?;
            (persisted_account_id, warning)
        };

        if existing_match.is_some() {
            updated_existing += 1;
        } else {
            created += 1;
        }
        results.push(ImportedOauthImportResult {
            source_id: normalized.source_id,
            file_name: normalized.file_name,
            email: Some(normalized.email),
            chatgpt_account_id: Some(normalized.chatgpt_account_id),
            account_id: Some(persisted_account_id),
            status: if existing_match.is_some() {
                IMPORT_RESULT_STATUS_UPDATED_EXISTING.to_string()
            } else {
                IMPORT_RESULT_STATUS_CREATED.to_string()
            },
            detail: import_warning,
            matched_account,
        });
    }

    Ok(Json(ImportedOauthImportResponse {
        summary: ImportedOauthImportSummary {
            input_files,
            selected_files,
            created,
            updated_existing,
            failed,
        },
        results,
    }))
}

pub(crate) async fn get_pool_routing_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PoolRoutingSettingsResponse>, (StatusCode, String)> {
    let row = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(build_pool_routing_settings_response(
        state.as_ref(),
        &row,
    )))
}

pub(crate) async fn update_pool_routing_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdatePoolRoutingSettingsRequest>,
) -> Result<Json<PoolRoutingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let current = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    let merged_maintenance = merge_pool_routing_maintenance_settings(
        resolve_pool_routing_maintenance_settings(&current, &state.config),
        payload.maintenance.as_ref(),
    );
    validate_pool_routing_maintenance_settings(merged_maintenance)?;

    let api_key = payload
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| normalize_required_secret(value, "apiKey"))
        .transpose()?;
    let timeout_updates = payload
        .timeouts
        .map(|timeouts| {
            Ok(UpdatePoolRoutingTimeoutSettingsRequest {
                responses_first_byte_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.responses_first_byte_timeout_secs,
                    "responsesFirstByteTimeoutSecs",
                )?,
                compact_first_byte_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.compact_first_byte_timeout_secs,
                    "compactFirstByteTimeoutSecs",
                )?,
                responses_stream_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.responses_stream_timeout_secs,
                    "responsesStreamTimeoutSecs",
                )?,
                compact_stream_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.compact_stream_timeout_secs,
                    "compactStreamTimeoutSecs",
                )?,
            })
        })
        .transpose()?;
    let crypto_key = if api_key.is_some() {
        Some(state.upstream_accounts.require_crypto_key()?)
    } else {
        None
    };
    if api_key.is_some() || timeout_updates.is_some() {
        save_pool_routing_settings(
            &state.pool,
            &state.config,
            crypto_key,
            api_key.as_deref(),
            timeout_updates.as_ref(),
        )
        .await?;
    }
    if payload.maintenance.is_some() {
        save_pool_routing_maintenance_settings(&state.pool, merged_maintenance)
            .await
            .map_err(internal_error_tuple)?;
    }
    let updated = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(build_pool_routing_settings_response(
        state.as_ref(),
        &updated,
    )))
}

pub(crate) async fn get_upstream_account_sticky_keys(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Query(params): Query<AccountStickyKeysQuery>,
) -> Result<Json<AccountStickyKeysResponse>, (StatusCode, String)> {
    ensure_hourly_rollups_caught_up(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let exists = load_upstream_account_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .is_some();
    if !exists {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    let limit = normalize_sticky_key_limit(params.limit);
    let response = build_account_sticky_keys_response(&state.pool, id, limit)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(response))
}

pub(crate) async fn create_oauth_mailbox_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOauthMailboxSessionRequest>,
) -> Result<Json<OauthMailboxSessionResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    cleanup_expired_oauth_mailbox_sessions(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let config = upstream_mailbox_config(&state.config)?;
    if let Some(manual_email_address) =
        match requested_manual_mailbox_address(payload.email_address.as_deref()) {
            RequestedManualMailboxAddress::Missing => None,
            RequestedManualMailboxAddress::Valid(value) => Some(value),
            RequestedManualMailboxAddress::Invalid(invalid_email_address) => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    invalid_email_address,
                    "invalid_format",
                )));
            }
        }
    {
        if !mailbox_address_is_valid(&manual_email_address) {
            return Ok(Json(oauth_mailbox_session_unsupported_response(
                manual_email_address,
                "invalid_format",
            )));
        }
        let moemail_config = moemail_get_config(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?;
        let supported_domains = moemail_supported_domains(&moemail_config);
        let email_domain = manual_email_address
            .split('@')
            .nth(1)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !supported_domains.is_empty() && !supported_domains.contains(&email_domain) {
            return Ok(Json(oauth_mailbox_session_unsupported_response(
                manual_email_address,
                "unsupported_domain",
            )));
        }
        let existing_remote_mailbox = moemail_list_emails(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?
            .into_iter()
            .find(|item| {
                normalize_mailbox_address(&item.address) == Some(manual_email_address.clone())
            });
        let Some(remote_mailbox) = existing_remote_mailbox else {
            let generated = moemail_create_email_for_address(
                &state.http_clients.shared,
                config,
                &manual_email_address,
            )
            .await
            .map_err(internal_error_tuple)?;
            let email_address = generated.email.trim().to_string();
            let email_domain = email_address
                .split('@')
                .nth(1)
                .unwrap_or(config.default_domain.as_str())
                .to_string();
            let session_id = random_hex(16)?;
            let now = Utc::now();
            let expires_at = now
                + ChronoDuration::seconds(
                    DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64,
                );
            let now_iso = format_utc_iso(now);
            let expires_at_iso = format_utc_iso(expires_at);
            sqlx::query(
                r#"
                INSERT INTO pool_oauth_mailbox_sessions (
                    session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
                    latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
                    invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
                    expires_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, NULL, ?6, ?6, ?7)
                "#,
            )
            .bind(&session_id)
            .bind(&generated.id)
            .bind(&email_address)
            .bind(&email_domain)
            .bind(OAUTH_MAILBOX_SOURCE_GENERATED)
            .bind(&now_iso)
            .bind(&expires_at_iso)
            .execute(&state.pool)
            .await
            .map_err(internal_error_tuple)?;

            return Ok(Json(oauth_mailbox_session_supported_response(
                session_id,
                email_address,
                expires_at_iso,
                OAUTH_MAILBOX_SOURCE_GENERATED,
            )));
        };
        let mut remote_messages = match moemail_list_messages_for_attach(
            &state.http_clients.shared,
            config,
            &remote_mailbox.id,
        )
        .await
        .map_err(internal_error_tuple)?
        {
            MoeMailAttachReadState::Readable(messages) => messages,
            MoeMailAttachReadState::NotReadable => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    manual_email_address,
                    "not_readable",
                )));
            }
        };
        sort_mailbox_messages_desc(&mut remote_messages);
        let latest_message_id = latest_mailbox_message_id(&remote_messages);
        let (latest_code, latest_invite) = match resolve_mailbox_message_state_for_attach(
            &state.http_clients.shared,
            config,
            &remote_mailbox.id,
            &remote_messages,
        )
        .await
        .map_err(internal_error_tuple)?
        {
            MoeMailAttachReadState::Readable(state) => state,
            MoeMailAttachReadState::NotReadable => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    manual_email_address,
                    "not_readable",
                )));
            }
        };
        let session_id = random_hex(16)?;
        let now = Utc::now();
        let expires_at = normalize_mailbox_session_expires_at(
            remote_mailbox.expires_at.as_deref(),
            now + ChronoDuration::seconds(
                DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64,
            ),
        );
        let now_iso = format_utc_iso(now);
        sqlx::query(
            r#"
            INSERT INTO pool_oauth_mailbox_sessions (
                session_id, remote_email_id, email_address, email_domain, mailbox_source,
                latest_code_value, latest_code_source, latest_code_updated_at, invite_subject,
                invite_copy_value, invite_copy_label, invite_updated_at, invited, last_message_id,
                created_at, updated_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?16)
            "#,
        )
        .bind(&session_id)
        .bind(&remote_mailbox.id)
        .bind(&manual_email_address)
        .bind(&email_domain)
        .bind(OAUTH_MAILBOX_SOURCE_ATTACHED)
        .bind(latest_code.as_ref().map(|value| value.value.clone()))
        .bind(latest_code.as_ref().map(|value| value.source.clone()))
        .bind(latest_code.as_ref().map(|value| value.updated_at.clone()))
        .bind(latest_invite.as_ref().map(|value| value.subject.clone()))
        .bind(latest_invite.as_ref().map(|value| value.copy_value.clone()))
        .bind(latest_invite.as_ref().map(|value| value.copy_label.clone()))
        .bind(latest_invite.as_ref().map(|value| value.updated_at.clone()))
        .bind(if latest_invite.is_some() { 1 } else { 0 })
        .bind(latest_message_id)
        .bind(&now_iso)
        .bind(&expires_at)
        .execute(&state.pool)
        .await
        .map_err(internal_error_tuple)?;

        return Ok(Json(oauth_mailbox_session_supported_response(
            session_id,
            manual_email_address,
            expires_at,
            OAUTH_MAILBOX_SOURCE_ATTACHED,
        )));
    }
    let generated = moemail_create_email(&state.http_clients.shared, config)
        .await
        .map_err(internal_error_tuple)?;
    let email_address = generated.email.trim().to_string();
    let email_domain = email_address
        .split('@')
        .nth(1)
        .unwrap_or(config.default_domain.as_str())
        .to_string();
    let session_id = random_hex(16)?;
    let now = Utc::now();
    let expires_at =
        now + ChronoDuration::seconds(DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64);
    let now_iso = format_utc_iso(now);
    let expires_at_iso = format_utc_iso(expires_at);
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_mailbox_sessions (
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, NULL, ?6, ?6, ?7)
        "#,
    )
    .bind(&session_id)
    .bind(&generated.id)
    .bind(&email_address)
    .bind(&email_domain)
    .bind(OAUTH_MAILBOX_SOURCE_GENERATED)
    .bind(&now_iso)
    .bind(&expires_at_iso)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple)?;

    Ok(Json(oauth_mailbox_session_supported_response(
        session_id,
        email_address,
        expires_at_iso,
        OAUTH_MAILBOX_SOURCE_GENERATED,
    )))
}

pub(crate) async fn get_oauth_mailbox_session_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<OauthMailboxStatusRequest>,
) -> Result<Json<OauthMailboxStatusBatchResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    cleanup_expired_oauth_mailbox_sessions(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let session_ids = payload
        .session_ids
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let rows = load_oauth_mailbox_sessions(&state.pool, &session_ids)
        .await
        .map_err(internal_error_tuple)?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        match refresh_oauth_mailbox_session_status(state.as_ref(), &row).await {
            Ok(refreshed) => items.push(oauth_mailbox_status_from_row(&refreshed)),
            Err(error) => {
                let mut status = oauth_mailbox_status_from_row(&row);
                status.error = Some(error.to_string());
                items.push(status);
            }
        }
    }
    Ok(Json(OauthMailboxStatusBatchResponse { items }))
}

pub(crate) async fn delete_oauth_mailbox_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let Some(row) = load_oauth_mailbox_session(&state.pool, &session_id)
        .await
        .map_err(internal_error_tuple)?
    else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if row.mailbox_source.as_deref() != Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
        && let Some(config) = state.config.upstream_accounts_moemail.as_ref()
        && let Err(err) =
            moemail_delete_email(&state.http_clients.shared, config, &row.remote_email_id).await
    {
        debug!(
            mailbox_session_id = %row.session_id,
            remote_email_id = %row.remote_email_id,
            error = %err,
            "failed to delete moemail mailbox during explicit cleanup"
        );
    }
    delete_oauth_mailbox_session_with_executor(&state.pool, &session_id)
        .await
        .map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOauthLoginSessionRequest>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    validate_mailbox_binding(
        &state.pool,
        payload.mailbox_session_id.as_deref(),
        payload.mailbox_address.as_deref(),
    )
    .await?;
    let tag_ids = validate_tag_ids(&state.pool, &payload.tag_ids).await?;
    let tag_ids_json = encode_tag_ids_json(&tag_ids).map_err(internal_error_tuple)?;

    let mut preserved_mother_flag = false;
    let mut preserved_display_name = None;
    let mut preserved_group_name = None;
    let mut preserved_note = None;

    if let Some(account_id) = payload.account_id {
        let Some(existing) = load_upstream_account_row(&state.pool, account_id)
            .await
            .map_err(internal_error_tuple)?
        else {
            return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
        };
        if existing.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
            return Err((
                StatusCode::BAD_REQUEST,
                "only OAuth accounts can be re-linked".to_string(),
            ));
        }
        preserved_mother_flag = existing.is_mother != 0;
        preserved_display_name = Some(existing.display_name);
        preserved_group_name = existing.group_name;
        preserved_note = existing.note;
    }

    let is_mother = payload.is_mother.unwrap_or(preserved_mother_flag);
    let display_name = normalize_optional_text(payload.display_name).or(preserved_display_name);
    let group_name = normalize_optional_text(payload.group_name).or(preserved_group_name);
    let note = normalize_optional_text(payload.note).or(preserved_note);
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        payload.group_bound_proxy_keys.clone(),
    )
    .await?;

    let redirect_uri = build_manual_callback_redirect_uri().map_err(internal_error_tuple)?;
    let login_id = random_hex(16)?;
    let state_token = random_hex(32)?;
    let pkce_verifier = random_hex(64)?;
    let code_challenge = code_challenge_for_verifier(&pkce_verifier);
    let auth_url = build_oauth_authorize_url(
        &state.config.upstream_accounts_oauth_issuer,
        &state.config.upstream_accounts_oauth_client_id,
        &redirect_uri,
        &state_token,
        &code_challenge,
    )
    .map_err(internal_error_tuple)?;
    let now = Utc::now();
    let expires_at = now
        + ChronoDuration::seconds(state.config.upstream_accounts_login_session_ttl.as_secs() as i64);
    let now_iso = format_utc_iso(now);
    let expires_at_iso = format_utc_iso(expires_at);
    let group_note = normalize_optional_text(payload.group_note.clone());
    validate_group_note_target(group_name.as_deref(), payload.group_note.is_some())?;
    let store_group_note = if payload.group_note.is_some() {
        if let Some(group_name) = group_name.as_deref() {
            !group_has_accounts(&state.pool, group_name)
                .await
                .map_err(internal_error_tuple)?
        } else {
            false
        }
    } else {
        false
    };
    let stored_group_note = if store_group_note { group_note } else { None };

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    if let Some(display_name) = display_name.as_deref() {
        ensure_display_name_available(&mut *tx, display_name, payload.account_id).await?;
    }

    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, group_name, group_bound_proxy_keys_json, is_mother, note, tag_ids_json, group_note,
            mailbox_session_id, generated_mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, expires_at, consumed_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, NULL, ?17, NULL, ?18, ?18)
        "#,
    )
    .bind(&login_id)
    .bind(payload.account_id)
    .bind(display_name)
    .bind(&resolved_group_binding.group_name)
    .bind(
        encode_group_bound_proxy_keys_json(&resolved_group_binding.bound_proxy_keys)
            .map_err(internal_error_tuple)?,
    )
    .bind(if is_mother { 1 } else { 0 })
    .bind(note)
    .bind(tag_ids_json)
    .bind(stored_group_note)
    .bind(normalize_optional_text(payload.mailbox_session_id.clone()))
    .bind(normalize_optional_text(payload.mailbox_address.clone()))
    .bind(&state_token)
    .bind(&pkce_verifier)
    .bind(&redirect_uri)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .bind(&auth_url)
    .bind(&expires_at_iso)
    .bind(&now_iso)
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;

    Ok(Json(LoginSessionStatusResponse {
        login_id,
        status: LOGIN_SESSION_STATUS_PENDING.to_string(),
        auth_url: Some(auth_url),
        redirect_uri: Some(redirect_uri),
        expires_at: expires_at_iso,
        updated_at: now_iso,
        account_id: payload.account_id,
        error: None,
        sync_applied: None,
    }))
}

pub(crate) async fn get_oauth_login_session(
    State(state): State<Arc<AppState>>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id(&state.pool, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    Ok(Json(login_session_to_response(&session)))
}

pub(crate) async fn update_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(login_id): AxumPath<String>,
    Json(payload): Json<UpdateOauthLoginSessionRequest>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id_with_executor(&mut *tx, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    let requested_base_updated_at = headers
        .get(LOGIN_SESSION_BASE_UPDATED_AT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let completed_race_repair_requested = session.status == LOGIN_SESSION_STATUS_COMPLETED
        && session.account_id.is_some()
        && session
            .consumed_at
            .as_deref()
            .is_some_and(|value| value != session.updated_at)
        && requested_base_updated_at
            .as_deref()
            .is_some_and(|value| value == session.updated_at);
    // Completed-session repairs are only valid for create-account sessions that
    // still preserve their last pending baseline after callback completion.
    // Relogin sessions advance updated_at when they complete, so they never
    // qualify for this narrow repair path.
    let allows_completed_race_repair = if completed_race_repair_requested {
        let account_id = session.account_id.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "completed session account is missing".to_string(),
            )
        })?;
        let account = load_upstream_account_row_conn(tx.as_mut(), account_id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
        let current_group_metadata = match session.group_name.as_deref() {
            Some(group_name) => load_group_metadata_conn(tx.as_mut(), group_name)
                .await
                .map_err(internal_error_tuple)?
                .unwrap_or_default(),
            None => UpstreamAccountGroupMetadata::default(),
        };
        let current_tag_ids = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        session.display_name.as_deref() == Some(account.display_name.as_str())
            && session.group_name == account.group_name
            && session.note == account.note
            && session.group_note == current_group_metadata.note
            && decode_group_bound_proxy_keys_json(session.group_bound_proxy_keys_json.as_deref())
                == current_group_metadata.bound_proxy_keys
            && (session.is_mother != 0) == (account.is_mother != 0)
            && parse_tag_ids_json(session.tag_ids_json.as_deref()) == current_tag_ids
    } else {
        false
    };
    if session.status != LOGIN_SESSION_STATUS_PENDING && !allows_completed_race_repair {
        return Err((
            StatusCode::BAD_REQUEST,
            if session.status == LOGIN_SESSION_STATUS_EXPIRED {
                "The login session has expired. Please create a new authorization link.".to_string()
            } else {
                "This login session can no longer be edited.".to_string()
            },
        ));
    }
    if session.account_id.is_some() && session.status == LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session belongs to an existing account and cannot be edited.".to_string(),
        ));
    }
    if session.status == LOGIN_SESSION_STATUS_PENDING {
        if let Some(requested_base_updated_at) = requested_base_updated_at.as_deref() {
            if requested_base_updated_at != session.updated_at {
                tx.commit().await.map_err(internal_error_tuple)?;
                return Ok(Json(login_session_to_response_with_sync_applied(
                    &session, false,
                )));
            }
        }
    }

    let UpdateOauthLoginSessionRequest {
        display_name: requested_display_name,
        group_name: requested_group_name,
        group_bound_proxy_keys: requested_group_bound_proxy_keys,
        note: requested_note,
        group_note: requested_group_note,
        tag_ids: requested_tag_ids,
        is_mother: requested_is_mother,
        mailbox_session_id: requested_mailbox_session_id,
        mailbox_address: requested_mailbox_address,
    } = payload;
    let requested_group_name_was_updated = !matches!(requested_group_name, OptionalField::Missing);
    let requested_group_bound_proxy_keys_was_updated =
        !matches!(requested_group_bound_proxy_keys, OptionalField::Missing);
    let requested_group_note_was_updated = !matches!(requested_group_note, OptionalField::Missing);

    let display_name = match requested_display_name {
        OptionalField::Missing => session.display_name.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let group_name = match requested_group_name {
        OptionalField::Missing => session.group_name.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let note = match requested_note {
        OptionalField::Missing => session.note.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let session_group_bound_proxy_keys =
        decode_group_bound_proxy_keys_json(session.group_bound_proxy_keys_json.as_deref());
    let requested_group_note_missing = matches!(requested_group_note, OptionalField::Missing);
    let mut normalized_group_note = match requested_group_note {
        OptionalField::Missing => session.group_note.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let group_name_changed = group_name.as_deref() != session.group_name.as_deref();
    let requested_group_bound_proxy_keys = match requested_group_bound_proxy_keys {
        OptionalField::Missing if group_name_changed => None,
        OptionalField::Missing => Some(session_group_bound_proxy_keys.clone()),
        OptionalField::Null => Some(Vec::new()),
        OptionalField::Value(value) => Some(normalize_bound_proxy_keys(value)),
    };
    if requested_group_name_was_updated
        && (group_name.is_none() || (requested_group_note_missing && group_name_changed))
    {
        normalized_group_note = None;
    }
    let mailbox_session_id = match requested_mailbox_session_id {
        OptionalField::Missing => session.mailbox_session_id.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let mailbox_address = match requested_mailbox_address {
        OptionalField::Missing => session.mailbox_address.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let requested_tag_ids = match requested_tag_ids {
        OptionalField::Missing => parse_tag_ids_json(session.tag_ids_json.as_deref()),
        OptionalField::Null => Vec::new(),
        OptionalField::Value(value) => value,
    };
    let tag_ids = validate_tag_ids(&state.pool, &requested_tag_ids).await?;
    let is_mother = match requested_is_mother {
        OptionalField::Missing => session.is_mother != 0,
        OptionalField::Null => false,
        OptionalField::Value(value) => value,
    };
    validate_mailbox_binding(
        &state.pool,
        mailbox_session_id.as_deref(),
        mailbox_address.as_deref(),
    )
    .await?;
    validate_group_note_target(group_name.as_deref(), normalized_group_note.is_some())?;
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        requested_group_bound_proxy_keys,
    )
    .await?;
    let tag_ids_json = encode_tag_ids_json(&tag_ids).map_err(internal_error_tuple)?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        normalized_group_note.clone(),
        requested_group_note_was_updated,
        Some(resolved_group_binding.bound_proxy_keys.clone()),
        requested_group_bound_proxy_keys_was_updated,
    );

    if display_name.as_deref() != session.display_name.as_deref() {
        if let Some(display_name) = display_name.as_deref() {
            ensure_display_name_available(&mut *tx, display_name, session.account_id).await?;
        }
    }

    let stored_group_note = if let Some(group_name) = group_name.as_deref() {
        if normalized_group_note.is_some()
            && group_has_accounts_conn(tx.as_mut(), group_name)
                .await
                .map_err(internal_error_tuple)?
        {
            None
        } else {
            normalized_group_note.clone()
        }
    } else {
        None
    };
    if allows_completed_race_repair {
        let account_id = session.account_id.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "completed session account is missing".to_string(),
            )
        })?;
        apply_oauth_login_session_metadata_to_account_with_executor(
            &mut tx,
            account_id,
            display_name.clone(),
            Some(resolved_group_binding.group_name.clone()),
            note.clone(),
            &requested_group_metadata_changes,
            is_mother,
            &tag_ids,
        )
        .await?;
        let completed_group_note_snapshot = load_group_note_snapshot_conn(
            tx.as_mut(),
            group_name.as_deref(),
            normalized_group_note.as_deref(),
        )
        .await
        .map_err(internal_error_tuple)?;
        let now_iso = next_login_session_updated_at(Some(&session.updated_at));
        sqlx::query(
            r#"
            UPDATE pool_oauth_login_sessions
            SET display_name = ?2,
                group_name = ?3,
                group_bound_proxy_keys_json = ?4,
                is_mother = ?5,
                note = ?6,
                tag_ids_json = ?7,
                group_note = ?8,
                mailbox_session_id = ?9,
                generated_mailbox_address = ?10,
                updated_at = ?11
            WHERE login_id = ?1
            "#,
        )
        .bind(&login_id)
        .bind(display_name)
        .bind(Some(resolved_group_binding.group_name.clone()))
        .bind(
            encode_group_bound_proxy_keys_json(&resolved_group_binding.bound_proxy_keys)
                .map_err(internal_error_tuple)?,
        )
        .bind(if is_mother { 1 } else { 0 })
        .bind(note)
        .bind(&tag_ids_json)
        .bind(completed_group_note_snapshot)
        .bind(mailbox_session_id)
        .bind(mailbox_address)
        .bind(&now_iso)
        .execute(&mut *tx)
        .await
        .map_err(internal_error_tuple)?;
        let updated = load_login_session_by_login_id_with_executor(&mut *tx, &login_id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
        tx.commit().await.map_err(internal_error_tuple)?;
        return Ok(Json(login_session_to_response_with_sync_applied(
            &updated, true,
        )));
    }
    let now_iso = next_login_session_updated_at(Some(&session.updated_at));
    let result = sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET display_name = ?2,
            group_name = ?3,
            group_bound_proxy_keys_json = ?4,
            is_mother = ?5,
            note = ?6,
            tag_ids_json = ?7,
            group_note = ?8,
            mailbox_session_id = ?9,
            generated_mailbox_address = ?10,
            updated_at = ?11
        WHERE login_id = ?1
          AND (?12 IS NULL OR updated_at = ?12)
        "#,
    )
    .bind(&login_id)
    .bind(display_name)
    .bind(Some(resolved_group_binding.group_name.clone()))
    .bind(
        encode_group_bound_proxy_keys_json(&resolved_group_binding.bound_proxy_keys)
            .map_err(internal_error_tuple)?,
    )
    .bind(if is_mother { 1 } else { 0 })
    .bind(note)
    .bind(tag_ids_json)
    .bind(stored_group_note)
    .bind(mailbox_session_id)
    .bind(mailbox_address)
    .bind(&now_iso)
    .bind(requested_base_updated_at.as_deref())
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    let updated = load_login_session_by_login_id_with_executor(&mut *tx, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    if result.rows_affected() == 0 {
        tx.commit().await.map_err(internal_error_tuple)?;
        return Ok(Json(login_session_to_response_with_sync_applied(
            &updated, false,
        )));
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(Json(login_session_to_response_with_sync_applied(
        &updated, true,
    )))
}

pub(crate) async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OauthCallbackQuery>,
) -> Response {
    match handle_oauth_callback(state, query).await {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err((status, html)) => (status, Html(html)).into_response(),
    }
}

pub(crate) async fn complete_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(login_id): AxumPath<String>,
    Json(payload): Json<CompleteOauthLoginSessionRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id(&state.pool, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    validate_mailbox_binding_fields(
        payload.mailbox_session_id.as_deref(),
        payload.mailbox_address.as_deref(),
    )?;
    if session.mailbox_session_id.as_deref() != payload.mailbox_session_id.as_deref()
        || !mailbox_addresses_match(
            session.mailbox_address.as_deref(),
            payload.mailbox_address.as_deref(),
        )
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "mailbox binding no longer matches this OAuth login session".to_string(),
        ));
    }
    validate_mailbox_binding(
        &state.pool,
        session.mailbox_session_id.as_deref(),
        session.mailbox_address.as_deref(),
    )
    .await?;
    let query = parse_manual_oauth_callback(&payload.callback_url, &session.redirect_uri)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    let account_id = complete_oauth_login_session_with_query(state.clone(), session, query).await?;
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "account not found after oauth completion".to_string(),
            )
        })?;
    Ok(Json(detail))
}

pub(crate) async fn relogin_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    let tag_ids = load_account_tag_map(&state.pool, &[id])
        .await
        .map_err(internal_error_tuple)?
        .remove(&id)
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.id)
        .collect();
    let payload = CreateOauthLoginSessionRequest {
        display_name: None,
        group_name: None,
        group_bound_proxy_keys: None,
        note: None,
        group_note: None,
        account_id: Some(id),
        tag_ids,
        is_mother: None,
        mailbox_session_id: None,
        mailbox_address: None,
    };
    create_oauth_login_session(State(state), headers, Json(payload)).await
}

async fn apply_mother_assignment(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    group_name: Option<&str>,
    is_mother: bool,
) -> Result<()> {
    if is_mother {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET is_mother = 0
            WHERE id != ?1
              AND COALESCE(group_name, '') = COALESCE(?2, '')
              AND is_mother != 0
            "#,
        )
        .bind(account_id)
        .bind(group_name)
        .execute(&mut **tx)
        .await?;
    }

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET is_mother = ?2
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(if is_mother { 1 } else { 0 })
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub(crate) async fn create_api_key_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateApiKeyAccountRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let detail = create_api_key_account_inner(state, payload).await?;
    Ok(Json(detail))
}

async fn create_api_key_account_inner(
    state: Arc<AppState>,
    payload: CreateApiKeyAccountRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let display_name = normalize_required_display_name(&payload.display_name)?;
    validate_local_limits(payload.local_primary_limit, payload.local_secondary_limit)?;
    let api_key = normalize_required_secret(&payload.api_key, "apiKey")?;
    let tag_ids = validate_tag_ids(&state.pool, &payload.tag_ids).await?;
    let group_name = normalize_optional_text(payload.group_name);
    let note = normalize_optional_text(payload.note);
    let has_group_note = payload.group_note.is_some();
    let group_note = normalize_optional_text(payload.group_note);
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        group_note.clone(),
        has_group_note,
        payload.group_bound_proxy_keys.clone(),
        payload.group_bound_proxy_keys.is_some(),
    );
    validate_group_note_target(group_name.as_deref(), has_group_note)?;
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        payload.group_bound_proxy_keys,
    )
    .await?;
    let target_group_name = Some(resolved_group_binding.group_name.clone());
    let is_mother = payload.is_mother.unwrap_or(false);
    let limit_unit = normalize_limit_unit(payload.local_limit_unit);
    let upstream_base_url = normalize_optional_upstream_base_url(payload.upstream_base_url)?;
    let masked_api_key = mask_api_key(&api_key);
    let now_iso = format_utc_iso(Utc::now());
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::ApiKey(StoredApiKeyCredentials { api_key }),
    )
    .map_err(internal_error_tuple)?;
    let inserted_id = {
        let mut tx = state
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(internal_error_tuple)?;
        ensure_display_name_available(&mut *tx, &display_name, None).await?;
        let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_upstream_accounts (
            kind, provider, display_name, group_name, is_mother, note, status, enabled, email, chatgpt_account_id,
            chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key, encrypted_credentials, token_expires_at,
            last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, NULL, NULL,
            NULL, NULL, NULL, ?8, ?9, NULL,
            NULL, NULL, NULL, NULL, NULL,
            ?10, ?11, ?12, ?13, ?14, ?14
        ) RETURNING id
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
    .bind(display_name)
    .bind(&target_group_name)
    .bind(if is_mother { 1 } else { 0 })
    .bind(note)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(masked_api_key)
    .bind(encrypted_credentials)
    .bind(payload.local_primary_limit)
    .bind(payload.local_secondary_limit)
    .bind(limit_unit)
    .bind(upstream_base_url)
    .bind(&now_iso)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
        apply_mother_assignment(&mut tx, inserted_id, group_name.as_deref(), is_mother)
            .await
            .map_err(internal_error_tuple)?;

        save_group_metadata_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            &requested_group_metadata_changes,
            false,
        )
        .await
        .map_err(internal_error_tuple)?;
        tx.commit().await.map_err(internal_error_tuple)?;
        inserted_id
    };

    sync_account_tag_links(&state.pool, inserted_id, &tag_ids)
        .await
        .map_err(internal_error_tuple)?;
    let detail = state
        .upstream_accounts
        .account_ops
        .run_post_create_sync(state.clone(), inserted_id)
        .await
        .map_err(internal_error_tuple)?;
    Ok(detail)
}

pub(crate) async fn update_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateUpstreamAccountRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(state.clone(), id, payload)
        .await?;
    Ok(Json(detail))
}

async fn update_upstream_account_inner(
    state: &AppState,
    id: i64,
    payload: UpdateUpstreamAccountRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let mut row = load_upstream_account_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let clear_hard_failure_after_update = row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && account_update_requests_manual_recovery(&payload)
        && route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        );
    let tag_ids = match payload.tag_ids.as_ref() {
        Some(values) => Some(validate_tag_ids(&state.pool, values).await?),
        None => None,
    };
    let previous_group_name = row.group_name.clone();
    let requested_group_note = payload
        .group_note
        .clone()
        .map(|value| normalize_optional_text(Some(value)));
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        requested_group_note.clone().flatten(),
        payload.group_note.is_some(),
        payload.group_bound_proxy_keys.clone(),
        payload.group_bound_proxy_keys.is_some(),
    );

    if let Some(display_name) = payload.display_name {
        row.display_name = normalize_required_display_name(&display_name)?;
    }
    if let Some(group_name) = payload.group_name.clone() {
        row.group_name = normalize_optional_text(Some(group_name));
    }
    if let Some(note) = payload.note {
        row.note = normalize_optional_text(Some(note));
    }
    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        match payload.upstream_base_url {
            OptionalField::Missing => {}
            OptionalField::Null => {
                row.upstream_base_url = None;
            }
            OptionalField::Value(value) => {
                row.upstream_base_url = normalize_optional_upstream_base_url(Some(value))?;
            }
        }
    }
    if let Some(enabled) = payload.enabled {
        row.enabled = if enabled { 1 } else { 0 };
    }
    if let Some(is_mother) = payload.is_mother {
        row.is_mother = if is_mother { 1 } else { 0 };
    }

    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        if let Some(api_key) = payload.api_key {
            let api_key = normalize_required_secret(&api_key, "apiKey")?;
            row.masked_api_key = Some(mask_api_key(&api_key));
            row.encrypted_credentials = Some(
                encrypt_credentials(
                    crypto_key,
                    &StoredCredentials::ApiKey(StoredApiKeyCredentials { api_key }),
                )
                .map_err(internal_error_tuple)?,
            );
        }
        if payload.local_primary_limit.is_some() {
            row.local_primary_limit = payload.local_primary_limit;
        }
        if payload.local_secondary_limit.is_some() {
            row.local_secondary_limit = payload.local_secondary_limit;
        }
        if payload.local_limit_unit.is_some() {
            row.local_limit_unit = Some(normalize_limit_unit(payload.local_limit_unit));
        }
        validate_local_limits(row.local_primary_limit, row.local_secondary_limit)?;
    }
    validate_group_note_target(row.group_name.as_deref(), requested_group_note.is_some())?;
    let resolved_group_binding =
        if payload.group_name.is_some() || payload.group_bound_proxy_keys.is_some() {
            Some(
                resolve_required_group_proxy_binding_for_write(
                    state,
                    row.group_name.clone(),
                    payload.group_bound_proxy_keys.clone(),
                )
                .await?,
            )
        } else {
            None
        };
    if let Some(resolved_group_binding) = resolved_group_binding.as_ref() {
        row.group_name = Some(resolved_group_binding.group_name.clone());
    }
    let now_iso = format_utc_iso(Utc::now());
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    ensure_display_name_available(&mut *tx, &row.display_name, Some(id)).await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            group_name = ?3,
            is_mother = ?4,
            note = ?5,
            enabled = ?6,
            masked_api_key = ?7,
            encrypted_credentials = ?8,
            local_primary_limit = ?9,
            local_secondary_limit = ?10,
            local_limit_unit = ?11,
            upstream_base_url = ?12,
            updated_at = ?13
        WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(&row.display_name)
    .bind(&row.group_name)
    .bind(row.is_mother)
    .bind(&row.note)
    .bind(row.enabled)
    .bind(&row.masked_api_key)
    .bind(&row.encrypted_credentials)
    .bind(row.local_primary_limit)
    .bind(row.local_secondary_limit)
    .bind(&row.local_limit_unit)
    .bind(&row.upstream_base_url)
    .bind(&now_iso)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;
    apply_mother_assignment(&mut tx, id, row.group_name.as_deref(), row.is_mother != 0)
        .await
        .map_err(internal_error_tuple)?;

    if previous_group_name == row.group_name {
        save_group_metadata_for_single_account_group(
            tx.as_mut(),
            row.group_name.as_deref(),
            &requested_group_metadata_changes,
        )
        .await
        .map_err(internal_error_tuple)?;
    } else {
        save_group_metadata_after_account_write(
            tx.as_mut(),
            row.group_name.as_deref(),
            &requested_group_metadata_changes,
            false,
        )
        .await
        .map_err(internal_error_tuple)?;
    }
    if previous_group_name != row.group_name {
        cleanup_orphaned_group_metadata(tx.as_mut(), previous_group_name.as_deref())
            .await
            .map_err(internal_error_tuple)?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Some(tag_ids) = tag_ids {
        sync_account_tag_links(&state.pool, id, &tag_ids)
            .await
            .map_err(internal_error_tuple)?;
    }
    if clear_hard_failure_after_update {
        set_account_status(&state.pool, id, UPSTREAM_ACCOUNT_STATUS_ACTIVE, None)
            .await
            .map_err(internal_error_tuple)?;
    }
    record_account_update_action(&state.pool, id, "account settings were updated")
        .await
        .map_err(internal_error_tuple)?;

    let detail = load_upstream_account_detail_with_actual_usage(state, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(detail)
}

async fn apply_oauth_login_session_metadata_to_account_with_executor(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    display_name: Option<String>,
    group_name: Option<String>,
    note: Option<String>,
    requested_group_metadata_changes: &RequestedGroupMetadataChanges,
    is_mother: bool,
    tag_ids: &[i64],
) -> Result<(), (StatusCode, String)> {
    let row = load_upstream_account_row_conn(tx.as_mut(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let previous_group_name = row.group_name.clone();
    let next_display_name = display_name.unwrap_or(row.display_name);
    ensure_display_name_available(tx.as_mut(), &next_display_name, Some(account_id)).await?;

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            group_name = ?3,
            is_mother = ?4,
            note = ?5,
            updated_at = ?6
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(&next_display_name)
    .bind(&group_name)
    .bind(if is_mother { 1 } else { 0 })
    .bind(&note)
    .bind(&now_iso)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;

    if previous_group_name == group_name {
        save_group_metadata_for_single_account_group(
            tx.as_mut(),
            group_name.as_deref(),
            requested_group_metadata_changes,
        )
        .await
        .map_err(internal_error_tuple)?;
    } else {
        save_group_metadata_after_account_write(
            tx.as_mut(),
            group_name.as_deref(),
            requested_group_metadata_changes,
            false,
        )
        .await
        .map_err(internal_error_tuple)?;
    }
    if previous_group_name != group_name {
        cleanup_orphaned_group_metadata(tx.as_mut(), previous_group_name.as_deref())
            .await
            .map_err(internal_error_tuple)?;
    }
    apply_mother_assignment(tx, account_id, group_name.as_deref(), is_mother)
        .await
        .map_err(internal_error_tuple)?;
    sync_account_tag_links_with_executor(tx.as_mut(), account_id, tag_ids)
        .await
        .map_err(internal_error_tuple)?;
    Ok(())
}

pub(crate) async fn delete_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let status = state
        .upstream_accounts
        .account_ops
        .run_delete_account(state.clone(), id)
        .await?;
    Ok(status)
}

async fn delete_upstream_account_inner(
    state: &AppState,
    id: i64,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let group_name = load_upstream_account_row_conn(tx.as_mut(), id)
        .await
        .map_err(internal_error_tuple)?
        .map(|row| row.group_name);
    sqlx::query("DELETE FROM pool_upstream_account_limit_samples WHERE account_id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    sqlx::query("DELETE FROM pool_upstream_account_tags WHERE account_id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    sqlx::query("DELETE FROM pool_oauth_login_sessions WHERE account_id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    let affected = sqlx::query("DELETE FROM pool_upstream_accounts WHERE id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?
        .rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    cleanup_orphaned_group_metadata(
        tx.as_mut(),
        group_name.as_ref().and_then(|value| value.as_deref()),
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn sync_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let detail = state
        .upstream_accounts
        .account_ops
        .run_manual_sync(state.clone(), id)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(detail))
}

async fn handle_oauth_callback(
    state: Arc<AppState>,
    query: OauthCallbackQuery,
) -> Result<String, (StatusCode, String)> {
    let Some(state_value) = normalize_optional_text(query.state.clone()) else {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(false, "OAuth callback rejected", "Missing state parameter."),
        ));
    };

    let Some(session) = load_login_session_by_state(&state.pool, &state_value)
        .await
        .map_err(internal_error_html)?
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(
                false,
                "OAuth callback rejected",
                "Login session was not found.",
            ),
        ));
    };

    complete_oauth_login_session_with_query(state, session, query)
        .await
        .map_err(|(status, message)| {
            let title = match status {
                StatusCode::BAD_GATEWAY => "OAuth token exchange failed",
                StatusCode::SERVICE_UNAVAILABLE => "Credential storage disabled",
                _ if message.contains("expired") => "OAuth callback expired",
                _ if message.contains("authorization failed") => "OAuth authorization failed",
                _ => "OAuth callback rejected",
            };
            (status, render_callback_page(false, title, &message))
        })?;

    Ok(render_callback_page(
        true,
        "OAuth login complete",
        "The upstream account is ready. You can close this window.",
    ))
}

async fn complete_oauth_login_session_with_query(
    state: Arc<AppState>,
    session: OauthLoginSessionRow,
    query: OauthCallbackQuery,
) -> Result<i64, (StatusCode, String)> {
    let now = Utc::now();
    let Some(expires_at) = parse_rfc3339_utc(&session.expires_at) else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Stored session expiry is invalid.".to_string(),
        ));
    };
    if session.status != LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session has already been consumed.".to_string(),
        ));
    }
    if now > expires_at {
        mark_login_session_expired(&state.pool, &session.login_id)
            .await
            .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_REQUEST,
            "The login session has expired. Please create a new authorization link.".to_string(),
        ));
    }

    let callback_state = normalize_optional_text(query.state.clone()).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing state parameter.".to_string(),
        )
    })?;
    if callback_state != session.state {
        return Err((
            StatusCode::BAD_REQUEST,
            "The callback URL does not belong to this login session.".to_string(),
        ));
    }

    if let Some(error) = normalize_optional_text(query.error) {
        let detail = normalize_optional_text(query.error_description)
            .unwrap_or_else(|| "Authorization was cancelled or rejected.".to_string());
        fail_login_session(
            &state.pool,
            &session.login_id,
            &format!("{error}: {detail}"),
        )
        .await
        .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_REQUEST,
            format!("OAuth authorization failed: {detail}"),
        ));
    }

    let code = normalize_optional_text(query.code).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing authorization code.".to_string(),
        )
    })?;

    let session_scope = login_session_required_forward_proxy_scope(&session)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    let token_response = exchange_authorization_code_for_required_scope(
        state.as_ref(),
        &session_scope,
        &code,
        &session.pkce_verifier,
        &session.redirect_uri,
    )
    .await
    .map_err(|err| (StatusCode::BAD_GATEWAY, err.to_string()))?;

    let Some(id_token) = token_response.id_token.clone() else {
        fail_login_session(
            &state.pool,
            &session.login_id,
            "id_token missing in token exchange response",
        )
        .await
        .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            "The token response did not include an id_token.".to_string(),
        ));
    };
    let Some(refresh_token) = token_response.refresh_token.clone() else {
        fail_login_session(
            &state.pool,
            &session.login_id,
            "refresh_token missing in token exchange response",
        )
        .await
        .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            "The token response did not include a refresh token.".to_string(),
        ));
    };

    let claims = parse_chatgpt_jwt_claims(&id_token)
        .map_err(|err| (StatusCode::BAD_GATEWAY, err.to_string()))?;
    let crypto_key = state.upstream_accounts.crypto_key.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "{} is required to persist OAuth credentials.",
                ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
            ),
        )
    })?;

    let token_expires_at =
        format_utc_iso(Utc::now() + ChronoDuration::seconds(token_response.expires_in.max(0)));
    let credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: token_response.access_token.clone(),
            refresh_token,
            id_token,
            token_type: token_response.token_type.clone(),
        }),
    )
    .map_err(internal_error_tuple)?;

    let default_display_name = claims
        .email
        .clone()
        .or_else(|| session.display_name.clone())
        .unwrap_or_else(|| "Codex OAuth".to_string());
    let display_name = session
        .display_name
        .clone()
        .and_then(|value| normalize_optional_text(Some(value)))
        .unwrap_or(default_display_name);
    let input = PersistOauthCallbackInput {
        session,
        display_name,
        claims,
        encrypted_credentials: credentials,
        token_expires_at,
    };
    let account_id = if let Some(existing_account_id) = input.session.account_id {
        state
            .upstream_accounts
            .account_ops
            .run_persist_oauth_callback(state.clone(), existing_account_id, input)
            .await?
    } else {
        let account_id = persist_new_oauth_callback_inner(state.as_ref(), input).await?;
        if let Err(err) = state
            .upstream_accounts
            .account_ops
            .run_post_create_sync(state.clone(), account_id)
            .await
        {
            warn!(account_id, error = %err, "OAuth callback created account but initial sync failed");
        }
        account_id
    };

    Ok(account_id)
}

async fn persist_existing_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    let account_id = persist_oauth_callback_inner(state, input).await?;
    if let Err(err) = sync_upstream_account_by_id(state, account_id, SyncCause::PostCreate).await {
        warn!(account_id, error = %err, "OAuth callback updated account but initial sync failed");
    }
    Ok(account_id)
}

async fn persist_new_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    persist_oauth_callback_inner(state, input).await
}

async fn persist_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id_with_executor(&mut *tx, &input.session.login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    if session.status != LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session has already been consumed.".to_string(),
        ));
    }
    if let Err((status, message)) =
        ensure_display_name_available(&mut *tx, &input.display_name, session.account_id).await
    {
        if status == StatusCode::CONFLICT {
            fail_login_session_with_executor(&mut *tx, &session.login_id, &message)
                .await
                .map_err(internal_error_tuple)?;
            tx.commit().await.map_err(internal_error_tuple)?;
        }
        return Err((status, message));
    }
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: session.account_id,
            display_name: &input.display_name,
            group_name: session.group_name.clone(),
            is_mother: session.is_mother != 0,
            note: session.note.clone(),
            tag_ids: parse_tag_ids_json(session.tag_ids_json.as_deref()),
            requested_group_metadata_changes: build_requested_group_metadata_changes(
                session.group_note.clone(),
                true,
                Some(decode_group_bound_proxy_keys_json(
                    session.group_bound_proxy_keys_json.as_deref(),
                )),
                true,
            ),
            claims: &input.claims,
            encrypted_credentials: input.encrypted_credentials,
            token_expires_at: &input.token_expires_at,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    let completed_group_note_snapshot = load_group_note_snapshot_conn(
        tx.as_mut(),
        session.group_name.as_deref(),
        session.group_note.as_deref(),
    )
    .await
    .map_err(internal_error_tuple)?;
    complete_login_session_with_executor(
        &mut *tx,
        &session.login_id,
        account_id,
        completed_group_note_snapshot,
        &session.updated_at,
        session.account_id.is_none(),
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(account_id)
}

fn parse_manual_oauth_callback(
    callback_url: &str,
    expected_redirect_uri: &str,
) -> Result<OauthCallbackQuery> {
    let trimmed = callback_url.trim();
    if trimmed.is_empty() {
        bail!("Callback URL is required.");
    }

    let expected =
        Url::parse(expected_redirect_uri).context("failed to parse stored redirect URI")?;
    let parsed = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Url::parse(trimmed).context("callback URL must be a valid absolute URL")?
    } else if trimmed.starts_with('?') || trimmed.contains("code=") || trimmed.contains("state=") {
        let mut url = expected.clone();
        let query = trimmed.strip_prefix('?').unwrap_or(trimmed);
        url.set_query(Some(query));
        url
    } else {
        bail!("Callback URL must be a full URL or query string.");
    };

    if parsed.scheme() != expected.scheme()
        || parsed.host_str() != expected.host_str()
        || parsed.port_or_known_default() != expected.port_or_known_default()
        || parsed.path() != expected.path()
    {
        bail!("Callback URL does not match the generated localhost redirect address.");
    }

    let mut query = OauthCallbackQuery {
        code: None,
        state: None,
        error: None,
        error_description: None,
    };
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" if query.code.is_none() => query.code = Some(value.into_owned()),
            "state" if query.state.is_none() => query.state = Some(value.into_owned()),
            "error" if query.error.is_none() => query.error = Some(value.into_owned()),
            "error_description" if query.error_description.is_none() => {
                query.error_description = Some(value.into_owned())
            }
            _ => {}
        }
    }
    Ok(query)
}

async fn run_upstream_account_maintenance_once(state: Arc<AppState>) -> Result<()> {
    expire_pending_login_sessions(&state.pool).await?;
    cleanup_expired_oauth_mailbox_sessions(state.as_ref()).await?;
    let Some(_) = state.upstream_accounts.crypto_key else {
        return Ok(());
    };
    let routing = load_pool_routing_settings(&state.pool).await?;
    let maintenance = resolve_pool_routing_maintenance_settings(&routing, &state.config);
    let candidates = load_maintenance_candidates(&state.pool).await?;
    let dispatch_plans = resolve_due_maintenance_dispatch_plans(
        candidates,
        maintenance,
        state.config.upstream_accounts_refresh_lead_time,
        Utc::now(),
    );

    let mut queued = 0usize;
    let mut deduped = 0usize;
    let mut failed = 0usize;
    let mut priority_due = 0usize;
    let mut secondary_due = 0usize;
    for plan in dispatch_plans {
        match plan.tier {
            MaintenanceTier::Priority => priority_due += 1,
            MaintenanceTier::Secondary => secondary_due += 1,
        }
        match state
            .upstream_accounts
            .account_ops
            .dispatch_maintenance_sync(state.clone(), plan)
        {
            Ok(MaintenanceQueueOutcome::Queued) => queued += 1,
            Ok(MaintenanceQueueOutcome::Deduped) => deduped += 1,
            Err(err) => {
                failed += 1;
                warn!(
                    account_id = plan.account_id,
                    tier = ?plan.tier,
                    error = %err,
                    "failed to dispatch upstream OAuth maintenance"
                );
            }
        }
    }

    info!(
        candidates = queued + deduped + failed,
        priority_due,
        secondary_due,
        queued,
        deduped,
        failed,
        "upstream account maintenance pass finished"
    );

    Ok(())
}

async fn ensure_integer_column_with_default(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
    default_value: &str,
) -> Result<()> {
    let pragma_statement = format!("PRAGMA table_info({table_name})");
    let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&pragma_statement).fetch_all(pool).await?;
    if columns
        .iter()
        .any(|(_, name, _, _, _, _)| name == column_name)
    {
        return Ok(());
    }

    let statement = format!(
        "ALTER TABLE {table_name} ADD COLUMN {column_name} INTEGER NOT NULL DEFAULT {default_value}"
    );
    sqlx::query(&statement).execute(pool).await?;

    Ok(())
}

async fn load_maintenance_candidates(pool: &Pool<Sqlite>) -> Result<Vec<MaintenanceCandidateRow>> {
    sqlx::query_as::<_, MaintenanceCandidateRow>(
        r#"
        SELECT
            account.id,
            account.status,
            account.last_synced_at,
            account.last_error_at,
            account.token_expires_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent
        FROM pool_upstream_accounts account
        WHERE account.kind = ?1
          AND account.enabled = 1
          AND account.status <> ?2
        ORDER BY account.id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn load_maintenance_candidate(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<MaintenanceCandidateRow>> {
    sqlx::query_as::<_, MaintenanceCandidateRow>(
        r#"
        SELECT
            account.id,
            account.status,
            account.last_synced_at,
            account.last_error_at,
            account.token_expires_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent
        FROM pool_upstream_accounts account
        WHERE account.id = ?1
          AND account.kind = ?2
          AND account.enabled = 1
          AND account.status <> ?3
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn maintenance_refresh_due(
    candidate: &MaintenanceCandidateRow,
    refresh_lead_time: Duration,
    now: DateTime<Utc>,
) -> bool {
    candidate
        .token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|expires| expires <= now + ChronoDuration::seconds(refresh_lead_time.as_secs() as i64))
        .unwrap_or(true)
}

fn maintenance_candidate_has_complete_usage(candidate: &MaintenanceCandidateRow) -> bool {
    candidate.primary_used_percent.is_some() && candidate.secondary_used_percent.is_some()
}

fn maintenance_candidate_is_available(candidate: &MaintenanceCandidateRow) -> bool {
    maintenance_candidate_has_complete_usage(candidate)
        && candidate.primary_used_percent.unwrap_or(100.0) < 100.0
        && candidate.secondary_used_percent.unwrap_or(100.0) < 100.0
}

fn maintenance_candidate_force_priority(
    candidate: &MaintenanceCandidateRow,
    refresh_lead_time: Duration,
    now: DateTime<Utc>,
) -> bool {
    candidate.status == UPSTREAM_ACCOUNT_STATUS_ERROR
        || maintenance_refresh_due(candidate, refresh_lead_time, now)
        || !maintenance_candidate_has_complete_usage(candidate)
}

fn compare_maintenance_candidates(
    lhs: &MaintenanceCandidateRow,
    rhs: &MaintenanceCandidateRow,
) -> std::cmp::Ordering {
    lhs.secondary_used_percent
        .unwrap_or(100.0)
        .partial_cmp(&rhs.secondary_used_percent.unwrap_or(100.0))
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            lhs.primary_used_percent
                .unwrap_or(100.0)
                .partial_cmp(&rhs.primary_used_percent.unwrap_or(100.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            lhs.last_synced_at
                .as_deref()
                .cmp(&rhs.last_synced_at.as_deref())
        })
        .then_with(|| lhs.id.cmp(&rhs.id))
}

fn maintenance_last_attempt_at(candidate: &MaintenanceCandidateRow) -> Option<DateTime<Utc>> {
    let last_synced_at = candidate
        .last_synced_at
        .as_deref()
        .and_then(parse_rfc3339_utc);
    if candidate.status != UPSTREAM_ACCOUNT_STATUS_ERROR {
        return last_synced_at;
    }

    let last_error_at = candidate
        .last_error_at
        .as_deref()
        .and_then(parse_rfc3339_utc);
    match (last_synced_at, last_error_at) {
        (Some(lhs), Some(rhs)) => Some(lhs.max(rhs)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn maintenance_interval_is_due(
    candidate: &MaintenanceCandidateRow,
    interval_secs: u64,
    now: DateTime<Utc>,
) -> bool {
    maintenance_last_attempt_at(candidate)
        .map(|last| now.signed_duration_since(last).num_seconds() >= interval_secs as i64)
        .unwrap_or(true)
}

fn maintenance_interval_for_tier(
    tier: MaintenanceTier,
    settings: PoolRoutingMaintenanceSettings,
) -> u64 {
    match tier {
        MaintenanceTier::Priority => settings.primary_sync_interval_secs,
        MaintenanceTier::Secondary => settings.secondary_sync_interval_secs,
    }
}

fn maintenance_plan_is_due(
    candidate: &MaintenanceCandidateRow,
    tier: MaintenanceTier,
    settings: PoolRoutingMaintenanceSettings,
    now: DateTime<Utc>,
) -> bool {
    maintenance_interval_is_due(
        candidate,
        maintenance_interval_for_tier(tier, settings),
        now,
    )
}

async fn execute_queued_maintenance_sync(
    state: &AppState,
    plan: MaintenanceDispatchPlan,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let Some(candidate) = load_maintenance_candidate(&state.pool, id).await? else {
        return Ok(None);
    };
    if !maintenance_interval_is_due(&candidate, plan.sync_interval_secs, Utc::now()) {
        return Ok(None);
    }

    sync_upstream_account_by_id(state, id, SyncCause::Maintenance).await
}

fn resolve_due_maintenance_dispatch_plans(
    candidates: Vec<MaintenanceCandidateRow>,
    settings: PoolRoutingMaintenanceSettings,
    refresh_lead_time: Duration,
    now: DateTime<Utc>,
) -> Vec<MaintenanceDispatchPlan> {
    let mut forced_priority = Vec::new();
    let mut ranked_available = Vec::new();
    let mut secondary = Vec::new();

    for candidate in candidates {
        if maintenance_candidate_force_priority(&candidate, refresh_lead_time, now) {
            forced_priority.push(candidate);
        } else if maintenance_candidate_is_available(&candidate) {
            ranked_available.push(candidate);
        } else {
            secondary.push(candidate);
        }
    }

    ranked_available.sort_by(compare_maintenance_candidates);
    forced_priority.sort_by(|lhs, rhs| lhs.id.cmp(&rhs.id));
    secondary.sort_by(compare_maintenance_candidates);

    let mut plans = Vec::new();
    for candidate in forced_priority {
        if maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier: MaintenanceTier::Priority,
                sync_interval_secs: settings.primary_sync_interval_secs,
            });
        }
    }
    for (index, candidate) in ranked_available.into_iter().enumerate() {
        let tier = if index < settings.priority_available_account_cap {
            MaintenanceTier::Priority
        } else {
            MaintenanceTier::Secondary
        };
        if maintenance_plan_is_due(&candidate, tier, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier,
                sync_interval_secs: match tier {
                    MaintenanceTier::Priority => settings.primary_sync_interval_secs,
                    MaintenanceTier::Secondary => settings.secondary_sync_interval_secs,
                },
            });
        }
    }
    for candidate in secondary {
        if maintenance_plan_is_due(&candidate, MaintenanceTier::Secondary, settings, now) {
            plans.push(MaintenanceDispatchPlan {
                account_id: candidate.id,
                tier: MaintenanceTier::Secondary,
                sync_interval_secs: settings.secondary_sync_interval_secs,
            });
        }
    }

    plans
}

async fn find_existing_import_match(
    pool: &Pool<Sqlite>,
    chatgpt_account_id: &str,
    email: &str,
) -> Result<Option<UpstreamAccountRow>> {
    let account_id_matches = sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE kind = ?1
          AND chatgpt_account_id = ?2
        ORDER BY updated_at DESC, id DESC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(chatgpt_account_id)
    .fetch_all(pool)
    .await?;
    if account_id_matches.len() > 1 {
        bail!(
            "multiple existing OAuth accounts match account_id {}",
            chatgpt_account_id
        );
    }
    if let Some(row) = account_id_matches.into_iter().next() {
        return Ok(Some(row));
    }

    let email_matches = sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE kind = ?1
          AND lower(trim(COALESCE(email, ''))) = lower(trim(?2))
        ORDER BY updated_at DESC, id DESC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(email)
    .fetch_all(pool)
    .await?;
    if email_matches.len() > 1 {
        bail!("multiple existing OAuth accounts match email {}", email);
    }
    Ok(email_matches.into_iter().next())
}

async fn probe_imported_oauth_credentials(
    state: &AppState,
    imported: &NormalizedImportedOauthCredentials,
    binding: &ResolvedRequiredGroupProxyBinding,
) -> Result<ImportedOauthProbeOutcome, anyhow::Error> {
    let scope = required_account_forward_proxy_scope(
        Some(&binding.group_name),
        binding.bound_proxy_keys.clone(),
    )?;
    let mut credentials = imported.credentials.clone();
    let mut claims = imported.claims.clone();
    let mut token_expires_at = imported.token_expires_at.clone();
    let expires_at = parse_rfc3339_utc(&token_expires_at);
    let refresh_due = expires_at
        .map(|expires| {
            expires
                <= Utc::now()
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);

    if refresh_due {
        let response =
            refresh_oauth_tokens_for_required_scope(state, &scope, &credentials.refresh_token)
                .await?;
        credentials.access_token = response.access_token;
        if let Some(refresh_token) = response.refresh_token {
            credentials.refresh_token = refresh_token;
        }
        if let Some(id_token) = response.id_token {
            credentials.id_token = id_token;
            claims = parse_chatgpt_jwt_claims(&credentials.id_token)?;
            claims.email = claims.email.or_else(|| Some(imported.email.clone()));
            claims.chatgpt_account_id = claims
                .chatgpt_account_id
                .or_else(|| Some(imported.chatgpt_account_id.clone()));
        }
        credentials.token_type = response.token_type;
        token_expires_at =
            format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)));
    }

    let usage_result = fetch_usage_snapshot_via_forward_proxy(
        state,
        &scope,
        &state.config,
        &credentials.access_token,
        claims
            .chatgpt_account_id
            .as_deref()
            .or(Some(imported.chatgpt_account_id.as_str())),
    )
    .await;
    let (snapshot, usage_snapshot_warning) = match usage_result {
        Ok(snapshot) => (Some(snapshot), None),
        Err(err) if is_import_invalid_error_message(&err.to_string()) => return Err(err),
        Err(err) if err.to_string().contains("401") || err.to_string().contains("403") => {
            let response =
                refresh_oauth_tokens_for_required_scope(state, &scope, &credentials.refresh_token)
                    .await?;
            credentials.access_token = response.access_token;
            if let Some(refresh_token) = response.refresh_token {
                credentials.refresh_token = refresh_token;
            }
            if let Some(id_token) = response.id_token {
                credentials.id_token = id_token;
                claims = parse_chatgpt_jwt_claims(&credentials.id_token)?;
                claims.email = claims.email.or_else(|| Some(imported.email.clone()));
                claims.chatgpt_account_id = claims
                    .chatgpt_account_id
                    .or_else(|| Some(imported.chatgpt_account_id.clone()));
            }
            credentials.token_type = response.token_type;
            token_expires_at =
                format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)));
            match fetch_usage_snapshot_via_forward_proxy(
                state,
                &scope,
                &state.config,
                &credentials.access_token,
                claims
                    .chatgpt_account_id
                    .as_deref()
                    .or(Some(imported.chatgpt_account_id.as_str())),
            )
            .await
            {
                Ok(snapshot) => (Some(snapshot), None),
                Err(retry_err)
                    if !is_import_invalid_error_message(&retry_err.to_string())
                        && !retry_err.to_string().contains("401")
                        && !retry_err.to_string().contains("403") =>
                {
                    (
                        None,
                        Some(format!(
                            "usage snapshot unavailable during validation: {retry_err}"
                        )),
                    )
                }
                Err(retry_err) => return Err(retry_err),
            }
        }
        Err(err) => (
            None,
            Some(format!(
                "usage snapshot unavailable during validation: {err}"
            )),
        ),
    };

    Ok(ImportedOauthProbeOutcome {
        token_expires_at,
        credentials,
        claims,
        usage_snapshot: snapshot.clone(),
        exhausted: snapshot
            .as_ref()
            .is_some_and(imported_snapshot_is_exhausted),
        usage_snapshot_warning,
    })
}

async fn sync_upstream_account_by_id(
    state: &AppState,
    id: i64,
    cause: SyncCause,
) -> Result<Option<UpstreamAccountDetail>> {
    let row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;

    if row.enabled == 0 {
        if cause == SyncCause::Manual {
            bail!("disabled accounts cannot be synced");
        }
        let detail = load_upstream_account_detail_with_actual_usage(state, id)
            .await?
            .ok_or_else(|| anyhow!("account not found"))?;
        return Ok(Some(detail));
    }

    match row.kind.as_str() {
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX => sync_oauth_account(state, &row, cause).await?,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX => {
            sync_api_key_account(&state.pool, &row, cause).await?
        }
        _ => bail!("unsupported account kind: {}", row.kind),
    }

    let detail = load_upstream_account_detail_with_actual_usage(state, id)
        .await?
        .ok_or_else(|| anyhow!("account not found after sync"))?;
    Ok(Some(detail))
}

async fn sync_api_key_account(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    cause: SyncCause,
) -> Result<()> {
    let sync_source = sync_cause_action_source(cause);
    if row.status != UPSTREAM_ACCOUNT_STATUS_ACTIVE
        && route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        )
    {
        let reason_message = if route_failure_kind_is_quota_exhausted(
            row.last_route_failure_kind.as_deref(),
        ) {
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset"
        } else {
            "manual recovery required because API key sync cannot verify whether upstream credentials or entitlements have recovered"
        };
        return record_account_sync_recovery_blocked(
            pool,
            row.id,
            sync_source,
            &row.status,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            reason_message,
            row.last_error.as_deref(),
            row.last_route_failure_kind.as_deref(),
        )
        .await;
    }
    mark_account_sync_success(
        pool,
        row.id,
        sync_source,
        if should_clear_route_failure_state_after_sync_success(row) {
            SyncSuccessRouteState::ClearFailureState
        } else {
            SyncSuccessRouteState::PreserveFailureState
        },
    )
    .await
}

async fn sync_oauth_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    cause: SyncCause,
) -> Result<()> {
    let sync_source = sync_cause_action_source(cause);
    set_account_status(&state.pool, row.id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None).await?;
    let now = Utc::now();
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .ok_or_else(|| anyhow!("account writes are disabled"))?;
    let decrypted = decrypt_credentials(
        crypto_key,
        row.encrypted_credentials
            .as_deref()
            .ok_or_else(|| anyhow!("missing encrypted OAuth credentials"))?,
    )?;
    let StoredCredentials::Oauth(mut credentials) = decrypted else {
        bail!("unexpected credential kind for OAuth account")
    };

    let expires_at = row.token_expires_at.as_deref().and_then(parse_rfc3339_utc);
    let refresh_due = expires_at
        .map(|expires| {
            expires
                <= now
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);
    let refresh_scope = match load_required_account_forward_proxy_scope_from_group_metadata(
        &state.pool,
        row.group_name.as_deref(),
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            record_classified_account_sync_failure(&state.pool, row, sync_source, &err.to_string())
                .await?;
            return Ok(());
        }
    };

    if refresh_due {
        match refresh_oauth_tokens_for_required_scope(
            state,
            &refresh_scope,
            &credentials.refresh_token,
        )
        .await
        {
            Ok(response) => {
                credentials.access_token = response.access_token;
                if let Some(refresh_token) = response.refresh_token {
                    credentials.refresh_token = refresh_token;
                }
                if let Some(id_token) = response.id_token {
                    credentials.id_token = id_token;
                }
                credentials.token_type = response.token_type;
                let token_expires_at = format_utc_iso(
                    Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                );
                persist_oauth_credentials(
                    &state.pool,
                    row.id,
                    crypto_key,
                    &credentials,
                    &token_expires_at,
                )
                .await?;
            }
            Err(err) if is_reauth_error(&err) => {
                record_account_sync_failure(
                    &state.pool,
                    row.id,
                    sync_source,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    &err.to_string(),
                    UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
                    None,
                    PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                    None,
                )
                .await?;
                return Ok(());
            }
            Err(err) => {
                let (disposition, reason_code, next_status, http_status, failure_kind) =
                    classify_sync_failure(&row.kind, &err.to_string());
                let next_status = match disposition {
                    UpstreamAccountFailureDisposition::HardUnavailable => {
                        next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR)
                    }
                    UpstreamAccountFailureDisposition::RateLimited
                    | UpstreamAccountFailureDisposition::Retryable => {
                        UPSTREAM_ACCOUNT_STATUS_ACTIVE
                    }
                };
                record_account_sync_failure(
                    &state.pool,
                    row.id,
                    sync_source,
                    next_status,
                    &err.to_string(),
                    reason_code,
                    http_status,
                    failure_kind,
                    None,
                )
                .await?;
                return Ok(());
            }
        }
    }

    let mut latest_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared during sync"))?;
    let decrypted = decrypt_credentials(
        crypto_key,
        latest_row
            .encrypted_credentials
            .as_deref()
            .ok_or_else(|| anyhow!("missing encrypted OAuth credentials"))?,
    )?;
    let StoredCredentials::Oauth(credentials) = decrypted else {
        bail!("unexpected credential kind for OAuth account")
    };

    let usage_scope = match load_required_account_forward_proxy_scope_from_group_metadata(
        &state.pool,
        latest_row.group_name.as_deref(),
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            record_classified_account_sync_failure(
                &state.pool,
                &latest_row,
                sync_source,
                &err.to_string(),
            )
            .await?;
            return Ok(());
        }
    };
    let usage_result = fetch_usage_snapshot_via_forward_proxy(
        state,
        &usage_scope,
        &state.config,
        &credentials.access_token,
        latest_row.chatgpt_account_id.as_deref(),
    )
    .await;

    let snapshot = match usage_result {
        Ok(snapshot) => snapshot,
        Err(err) if err.to_string().contains("401") || err.to_string().contains("403") => {
            match refresh_oauth_tokens_for_required_scope(
                state,
                &usage_scope,
                &credentials.refresh_token,
            )
            .await
            {
                Ok(response) => {
                    let mut refreshed = credentials.clone();
                    refreshed.access_token = response.access_token;
                    if let Some(refresh_token) = response.refresh_token {
                        refreshed.refresh_token = refresh_token;
                    }
                    if let Some(id_token) = response.id_token {
                        refreshed.id_token = id_token;
                    }
                    refreshed.token_type = response.token_type;
                    let token_expires_at = format_utc_iso(
                        Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                    );
                    persist_oauth_credentials(
                        &state.pool,
                        row.id,
                        crypto_key,
                        &refreshed,
                        &token_expires_at,
                    )
                    .await?;
                    latest_row = load_upstream_account_row(&state.pool, row.id)
                        .await?
                        .ok_or_else(|| anyhow!("account disappeared during retry refresh"))?;
                    match fetch_usage_snapshot_via_forward_proxy(
                        state,
                        &usage_scope,
                        &state.config,
                        &refreshed.access_token,
                        latest_row.chatgpt_account_id.as_deref(),
                    )
                    .await
                    {
                        Ok(snapshot) => snapshot,
                        Err(retry_err) => {
                            record_classified_account_sync_failure(
                                &state.pool,
                                &latest_row,
                                sync_source,
                                &retry_err.to_string(),
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                }
                Err(refresh_err) if is_reauth_error(&refresh_err) => {
                    record_account_sync_failure(
                        &state.pool,
                        row.id,
                        sync_source,
                        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                        &refresh_err.to_string(),
                        UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
                        None,
                        PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                Err(refresh_err) => {
                    record_classified_account_sync_failure(
                        &state.pool,
                        &latest_row,
                        sync_source,
                        &refresh_err.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
        Err(err) => {
            record_classified_account_sync_failure(
                &state.pool,
                &latest_row,
                sync_source,
                &err.to_string(),
            )
            .await?;
            return Ok(());
        }
    };

    let effective_snapshot_plan_type =
        resolve_snapshot_plan_type(&state.pool, &latest_row, &snapshot).await?;
    persist_usage_snapshot(
        &state.pool,
        latest_row.id,
        effective_snapshot_plan_type.as_deref(),
        &snapshot,
        state.config.upstream_accounts_history_retention_days,
    )
    .await?;
    let latest_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared after usage snapshot persisted"))?;
    if route_failure_kind_is_quota_exhausted(latest_row.last_route_failure_kind.as_deref())
        && imported_snapshot_is_exhausted(&snapshot)
    {
        record_account_sync_recovery_blocked(
            &state.pool,
            row.id,
            sync_source,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
            "latest usage snapshot still shows an exhausted upstream usage limit window",
            latest_row
                .last_error
                .as_deref()
                .or(row.last_error.as_deref()),
            latest_row.last_route_failure_kind.as_deref(),
        )
        .await?;
        return Ok(());
    }
    if imported_snapshot_is_exhausted(&snapshot) {
        record_account_sync_hard_unavailable(
            &state.pool,
            row.id,
            sync_source,
            UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
            "latest usage snapshot already shows an exhausted upstream usage limit window",
            PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED,
        )
        .await?;
        return Ok(());
    }
    mark_account_sync_success(
        &state.pool,
        row.id,
        sync_source,
        if should_clear_route_failure_state_after_sync_success(&latest_row) {
            SyncSuccessRouteState::ClearFailureState
        } else {
            SyncSuccessRouteState::PreserveFailureState
        },
    )
    .await?;
    Ok(())
}

async fn persist_oauth_credentials(
    pool: &Pool<Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
) -> Result<()> {
    let claims = parse_chatgpt_jwt_claims(&credentials.id_token).unwrap_or_default();
    let encrypted =
        encrypt_credentials(crypto_key, &StoredCredentials::Oauth(credentials.clone()))?;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET encrypted_credentials = ?2,
            token_expires_at = ?3,
            last_refreshed_at = ?4,
            email = COALESCE(?5, email),
            chatgpt_account_id = COALESCE(?6, chatgpt_account_id),
            chatgpt_user_id = COALESCE(?7, chatgpt_user_id),
            plan_type = COALESCE(?8, plan_type),
            plan_type_observed_at = CASE
                WHEN NULLIF(TRIM(?8), '') IS NOT NULL THEN ?4
                ELSE plan_type_observed_at
            END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(encrypted)
    .bind(token_expires_at)
    .bind(&now_iso)
    .bind(claims.email)
    .bind(claims.chatgpt_account_id)
    .bind(claims.chatgpt_user_id)
    .bind(claims.chatgpt_plan_type)
    .execute(pool)
    .await?;
    Ok(())
}

async fn persist_usage_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    effective_plan_type: Option<&str>,
    snapshot: &NormalizedUsageSnapshot,
    retention_days: u64,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .bind(&snapshot.limit_id)
    .bind(&snapshot.limit_name)
    .bind(
        snapshot
            .plan_type
            .clone()
            .or_else(|| effective_plan_type.map(str::to_string)),
    )
    .bind(snapshot.primary.as_ref().map(|value| value.used_percent))
    .bind(
        snapshot
            .primary
            .as_ref()
            .map(|value| value.window_duration_mins),
    )
    .bind(
        snapshot
            .primary
            .as_ref()
            .and_then(|value| value.resets_at.clone()),
    )
    .bind(snapshot.secondary.as_ref().map(|value| value.used_percent))
    .bind(
        snapshot
            .secondary
            .as_ref()
            .map(|value| value.window_duration_mins),
    )
    .bind(
        snapshot
            .secondary
            .as_ref()
            .and_then(|value| value.resets_at.clone()),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .map(|value| if value.has_credits { 1 } else { 0 }),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .map(|value| if value.unlimited { 1 } else { 0 }),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .and_then(|value| value.balance.clone()),
    )
    .execute(pool)
    .await?;

    let retention_cutoff = format_utc_iso(Utc::now() - ChronoDuration::days(retention_days as i64));
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1 AND captured_at < ?2
        "#,
    )
    .bind(account_id)
    .bind(retention_cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

async fn apply_imported_oauth_probe_result(
    state: &AppState,
    account_id: i64,
    probe: &ImportedOauthProbeOutcome,
) -> Result<Option<String>> {
    if let Some(snapshot) = probe.usage_snapshot.as_ref() {
        persist_usage_snapshot(
            &state.pool,
            account_id,
            probe.claims.chatgpt_plan_type.as_deref(),
            snapshot,
            state.config.upstream_accounts_history_retention_days,
        )
        .await?;
        mark_account_sync_success(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT,
            SyncSuccessRouteState::ClearFailureState,
        )
        .await?;
    }
    Ok(probe.usage_snapshot_warning.clone())
}

async fn persist_imported_oauth_existing_inner(
    state: &AppState,
    account_id: i64,
    probe: ImportedOauthProbeOutcome,
) -> Result<Option<String>, (StatusCode, String)> {
    let existing_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let existing_tag_ids = load_account_tag_map(&state.pool, &[account_id])
        .await
        .map_err(internal_error_tuple)?
        .remove(&account_id)
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.id)
        .collect::<Vec<_>>();
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(probe.credentials.clone()),
    )
    .map_err(internal_error_tuple)?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    ensure_display_name_available(&mut *tx, &existing_row.display_name, Some(existing_row.id))
        .await?;
    upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(existing_row.id),
            display_name: &existing_row.display_name,
            group_name: existing_row.group_name.clone(),
            is_mother: existing_row.is_mother != 0,
            note: existing_row.note.clone(),
            tag_ids: existing_tag_ids,
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &probe.claims,
            encrypted_credentials,
            token_expires_at: &probe.token_expires_at,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;

    match apply_imported_oauth_probe_result(state, account_id, &probe).await {
        Ok(warning) => Ok(warning),
        Err(err) => {
            warn!(
                account_id,
                error = %err,
                "imported OAuth credential persisted but post-import state update failed"
            );
            Ok(Some(format!(
                "Imported, but post-import state update failed: {err}"
            )))
        }
    }
}

struct OauthAccountUpsert<'a> {
    account_id: Option<i64>,
    display_name: &'a str,
    group_name: Option<String>,
    is_mother: bool,
    note: Option<String>,
    tag_ids: Vec<i64>,
    requested_group_metadata_changes: RequestedGroupMetadataChanges,
    claims: &'a ChatgptJwtClaims,
    encrypted_credentials: String,
    token_expires_at: &'a str,
}

fn duplicate_display_name_error() -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        "displayName must be unique".to_string(),
    )
}

async fn load_conflicting_display_name_id(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE lower(trim(display_name)) = lower(trim(?1))
          AND (?2 IS NULL OR id != ?2)
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(display_name)
    .bind(exclude_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

async fn ensure_display_name_available(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
) -> Result<(), (StatusCode, String)> {
    let conflict = load_conflicting_display_name_id(executor, display_name, exclude_id)
        .await
        .map_err(internal_error_tuple)?;
    if conflict.is_some() {
        return Err(duplicate_display_name_error());
    }
    Ok(())
}

async fn upsert_oauth_account(
    tx: &mut Transaction<'_, Sqlite>,
    payload: OauthAccountUpsert<'_>,
) -> Result<i64> {
    let OauthAccountUpsert {
        account_id,
        display_name,
        group_name,
        is_mother,
        note,
        tag_ids,
        requested_group_metadata_changes,
        claims,
        encrypted_credentials,
        token_expires_at,
    } = payload;
    let target_group_name = group_name.clone();
    let now_iso = format_utc_iso(Utc::now());
    let resolved_account_id = account_id;

    if let Some(existing_id) = resolved_account_id {
        let previous_group_name = load_upstream_account_row_conn(tx.as_mut(), existing_id)
            .await?
            .and_then(|row| row.group_name);
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET kind = ?2,
                provider = ?3,
                display_name = ?4,
                group_name = COALESCE(?5, group_name),
                is_mother = ?6,
                note = ?7,
                status = ?8,
                enabled = 1,
                email = ?9,
                chatgpt_account_id = ?10,
                chatgpt_user_id = ?11,
                plan_type = ?12,
                plan_type_observed_at = CASE
                    WHEN NULLIF(TRIM(?12), '') IS NOT NULL THEN ?15
                    ELSE plan_type_observed_at
                END,
                encrypted_credentials = ?13,
                token_expires_at = ?14,
                last_refreshed_at = ?15,
                last_error = NULL,
                last_error_at = NULL,
                updated_at = ?15
            WHERE id = ?1
            "#,
        )
        .bind(existing_id)
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(&group_name)
        .bind(if is_mother { 1 } else { 0 })
        .bind(note)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(claims.email.clone())
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .execute(tx.as_mut())
        .await?;
        save_group_metadata_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            &requested_group_metadata_changes,
            previous_group_name == target_group_name,
        )
        .await?;
        if previous_group_name != target_group_name {
            cleanup_orphaned_group_metadata(tx.as_mut(), previous_group_name.as_deref()).await?;
        }
        apply_mother_assignment(tx, existing_id, group_name.as_deref(), is_mother).await?;
        sync_account_tag_links_with_executor(tx.as_mut(), existing_id, &tag_ids).await?;
        Ok(existing_id)
    } else {
        let inserted_account_id: i64 = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, is_mother, note, status, enabled,
                email, chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at,
                masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at,
                last_error, last_error_at, local_primary_limit, local_secondary_limit,
                local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1,
                ?8, ?9, ?10, ?11, ?12,
                NULL, ?13, ?14,
                ?15, NULL, NULL,
                NULL, NULL, NULL, NULL,
                NULL, ?15, ?15
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(&group_name)
        .bind(if is_mother { 1 } else { 0 })
        .bind(note)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(claims.email.clone())
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(
            claims
                .chatgpt_plan_type
                .as_deref()
                .and_then(|value| (!value.trim().is_empty()).then_some(now_iso.clone())),
        )
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .fetch_one(tx.as_mut())
        .await?;
        save_group_metadata_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            &requested_group_metadata_changes,
            false,
        )
        .await?;
        apply_mother_assignment(tx, inserted_account_id, group_name.as_deref(), is_mother).await?;
        sync_account_tag_links_with_executor(tx.as_mut(), inserted_account_id, &tag_ids).await?;
        Ok(inserted_account_id)
    }
}

#[derive(Debug, FromRow)]
struct UpstreamAccountIdentityRow {
    id: i64,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone)]
struct UpstreamAccountIdentityClusterMember {
    id: i64,
    plan_type: Option<String>,
}

fn is_team_plan_type(plan_type: Option<&str>) -> bool {
    plan_type
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("team"))
}

fn normalize_plan_type(plan_type: Option<&str>) -> Option<String> {
    plan_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_effective_plan_type(
    account_plan_type: Option<&str>,
    sample_plan_type: Option<&str>,
) -> Option<String> {
    normalize_plan_type(sample_plan_type).or_else(|| normalize_plan_type(account_plan_type))
}

async fn resolve_snapshot_plan_type(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    snapshot: &NormalizedUsageSnapshot,
) -> Result<Option<String>> {
    if let Some(plan_type) = normalize_plan_type(snapshot.plan_type.as_deref()) {
        return Ok(Some(plan_type));
    }

    let latest_sample_plan_type = load_latest_usage_sample(pool, row.id)
        .await?
        .and_then(|sample| sample.plan_type);
    Ok(latest_sample_plan_type.or_else(|| normalize_plan_type(row.plan_type.as_deref())))
}

async fn load_duplicate_info_map(
    pool: &Pool<Sqlite>,
) -> Result<std::collections::HashMap<i64, DuplicateInfo>> {
    let rows = sqlx::query_as::<_, UpstreamAccountIdentityRow>(
        r#"
        SELECT
            account.id,
            account.chatgpt_account_id,
            account.chatgpt_user_id,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = account.id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = account.id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type
        FROM pool_upstream_accounts account
        WHERE account.kind = ?1
        ORDER BY id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .fetch_all(pool)
    .await?;

    let mut by_account_id =
        std::collections::HashMap::<String, Vec<UpstreamAccountIdentityClusterMember>>::new();
    let mut by_user_id = std::collections::HashMap::<String, Vec<i64>>::new();
    for row in &rows {
        if let Some(chatgpt_account_id) = row.chatgpt_account_id.as_ref().cloned() {
            by_account_id.entry(chatgpt_account_id).or_default().push(
                UpstreamAccountIdentityClusterMember {
                    id: row.id,
                    plan_type: row.plan_type.clone(),
                },
            );
        }
        if let Some(chatgpt_user_id) = row.chatgpt_user_id.as_ref().cloned() {
            by_user_id.entry(chatgpt_user_id).or_default().push(row.id);
        }
    }

    let mut duplicate_info = std::collections::HashMap::new();
    for row in rows {
        let mut peer_ids = std::collections::BTreeSet::new();
        let mut reasons = Vec::new();

        if let Some(chatgpt_account_id) = row.chatgpt_account_id.as_ref()
            && let Some(cluster) = by_account_id
                .get(chatgpt_account_id)
                .filter(|members| members.len() > 1)
        {
            let is_all_team_cluster = cluster
                .iter()
                .all(|member| is_team_plan_type(member.plan_type.as_deref()));
            if !is_all_team_cluster {
                for member in cluster {
                    if member.id != row.id {
                        peer_ids.insert(member.id);
                    }
                }
                if !peer_ids.is_empty() {
                    reasons.push(DuplicateReason::SharedChatgptAccountId);
                }
            }
        }

        if let Some(chatgpt_user_id) = row.chatgpt_user_id.as_ref()
            && let Some(ids) = by_user_id.get(chatgpt_user_id).filter(|ids| ids.len() > 1)
        {
            for peer_id in ids {
                if *peer_id != row.id {
                    peer_ids.insert(*peer_id);
                }
            }
            if ids.iter().any(|peer_id| *peer_id != row.id) {
                reasons.push(DuplicateReason::SharedChatgptUserId);
            }
        }

        if !peer_ids.is_empty() {
            duplicate_info.insert(
                row.id,
                DuplicateInfo {
                    peer_account_ids: peer_ids.into_iter().collect(),
                    reasons,
                },
            );
        }
    }

    Ok(duplicate_info)
}

async fn load_account_tag_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, Vec<AccountTagSummary>>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            link.account_id,
            tag.id AS tag_id,
            tag.name,
            tag.guard_enabled,
            tag.lookback_hours,
            tag.max_conversations,
            tag.allow_cut_out,
            tag.allow_cut_in
        FROM pool_upstream_account_tags link
        INNER JOIN pool_tags tag ON tag.id = link.tag_id
        WHERE link.account_id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(") ORDER BY tag.name COLLATE NOCASE ASC, tag.id ASC")
        .build_query_as::<AccountTagRow>()
        .fetch_all(pool)
        .await?;
    let mut grouped: HashMap<i64, Vec<AccountTagSummary>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.account_id)
            .or_default()
            .push(account_tag_summary_from_row(&row));
    }
    Ok(grouped)
}

async fn load_tags_by_ids(pool: &Pool<Sqlite>, tag_ids: &[i64]) -> Result<Vec<TagRow>> {
    if tag_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            name,
            guard_enabled,
            lookback_hours,
            max_conversations,
            allow_cut_out,
            allow_cut_in
        FROM pool_tags
        WHERE id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for tag_id in tag_ids {
            separated.push_bind(tag_id);
        }
    }
    query
        .push(") ORDER BY name COLLATE NOCASE ASC, id ASC")
        .build_query_as::<TagRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn load_tag_row(pool: &Pool<Sqlite>, tag_id: i64) -> Result<Option<TagRow>> {
    sqlx::query_as::<_, TagRow>(
        r#"
        SELECT
            name,
            guard_enabled,
            lookback_hours,
            max_conversations,
            allow_cut_out,
            allow_cut_in
        FROM pool_tags
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(tag_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_tag_detail(pool: &Pool<Sqlite>, tag_id: i64) -> Result<Option<TagDetail>> {
    let items = load_tag_summaries(
        pool,
        &ListTagsQuery {
            search: None,
            has_accounts: None,
            guard_enabled: None,
            allow_cut_in: None,
            allow_cut_out: None,
        },
    )
    .await?;
    Ok(items
        .into_iter()
        .find(|item| item.id == tag_id)
        .map(|summary| TagDetail { summary }))
}

async fn load_tag_summaries(
    pool: &Pool<Sqlite>,
    params: &ListTagsQuery,
) -> Result<Vec<TagSummary>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            tag.id,
            tag.name,
            tag.guard_enabled,
            tag.lookback_hours,
            tag.max_conversations,
            tag.allow_cut_out,
            tag.allow_cut_in,
            tag.updated_at,
            COUNT(DISTINCT link.account_id) AS account_count,
            COUNT(DISTINCT NULLIF(TRIM(account.group_name), '')) AS group_count
        FROM pool_tags tag
        LEFT JOIN pool_upstream_account_tags link ON link.tag_id = tag.id
        LEFT JOIN pool_upstream_accounts account ON account.id = link.account_id
        WHERE 1 = 1
        "#,
    );
    if let Some(search) = params
        .search
        .as_ref()
        .and_then(|value| normalize_optional_text(Some(value.clone())))
    {
        query
            .push(" AND tag.name LIKE ")
            .push_bind(format!("%{search}%"));
    }
    if let Some(guard_enabled) = params.guard_enabled {
        query
            .push(" AND tag.guard_enabled = ")
            .push_bind(if guard_enabled { 1 } else { 0 });
    }
    if let Some(allow_cut_in) = params.allow_cut_in {
        query
            .push(" AND tag.allow_cut_in = ")
            .push_bind(if allow_cut_in { 1 } else { 0 });
    }
    if let Some(allow_cut_out) = params.allow_cut_out {
        query
            .push(" AND tag.allow_cut_out = ")
            .push_bind(if allow_cut_out { 1 } else { 0 });
    }
    query.push(
        " GROUP BY tag.id, tag.name, tag.guard_enabled, tag.lookback_hours, tag.max_conversations, tag.allow_cut_out, tag.allow_cut_in, tag.updated_at",
    );
    if let Some(has_accounts) = params.has_accounts {
        query.push(if has_accounts {
            " HAVING COUNT(DISTINCT link.account_id) > 0"
        } else {
            " HAVING COUNT(DISTINCT link.account_id) = 0"
        });
    }
    let rows = query
        .push(" ORDER BY tag.updated_at DESC, tag.id DESC")
        .build_query_as::<TagListRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| tag_summary_from_row(&row))
        .collect())
}

async fn insert_tag(pool: &Pool<Sqlite>, name: &str, rule: &TagRoutingRule) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations, allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(if rule.guard_enabled { 1 } else { 0 })
    .bind(rule.lookback_hours)
    .bind(rule.max_conversations)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(&now_iso)
    .fetch_one(pool)
    .await?;
    load_tag_detail(pool, inserted_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after insert"))
}

async fn persist_tag_update(
    pool: &Pool<Sqlite>,
    tag_id: i64,
    name: &str,
    rule: &TagRoutingRule,
) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_tags
        SET name = ?2,
            guard_enabled = ?3,
            lookback_hours = ?4,
            max_conversations = ?5,
            allow_cut_out = ?6,
            allow_cut_in = ?7,
            updated_at = ?8
        WHERE id = ?1
        "#,
    )
    .bind(tag_id)
    .bind(name)
    .bind(if rule.guard_enabled { 1 } else { 0 })
    .bind(rule.lookback_hours)
    .bind(rule.max_conversations)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(&now_iso)
    .execute(pool)
    .await?;
    load_tag_detail(pool, tag_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after update"))
}

async fn delete_tag_by_id(pool: &Pool<Sqlite>, tag_id: i64) -> Result<(), (StatusCode, String)> {
    let linked_account_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_account_tags WHERE tag_id = ?1",
    )
    .bind(tag_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error_tuple)?;
    let linked_session_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_oauth_login_sessions
        WHERE tag_ids_json IS NOT NULL
          AND EXISTS (
              SELECT 1
              FROM json_each(pool_oauth_login_sessions.tag_ids_json)
              WHERE CAST(json_each.value AS INTEGER) = ?1
          )
        "#,
    )
    .bind(tag_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error_tuple)?;
    if linked_account_count > 0 || linked_session_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            "tag is still associated with accounts or pending OAuth sessions".to_string(),
        ));
    }
    let affected = sqlx::query("DELETE FROM pool_tags WHERE id = ?1")
        .bind(tag_id)
        .execute(pool)
        .await
        .map_err(internal_error_tuple)?
        .rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "tag not found".to_string()));
    }
    Ok(())
}

fn map_tag_write_error(err: anyhow::Error) -> (StatusCode, String) {
    let message = err.to_string();
    if message.contains("UNIQUE constraint failed") {
        (StatusCode::CONFLICT, "tag name already exists".to_string())
    } else {
        internal_error_tuple(err)
    }
}

async fn validate_tag_ids(
    pool: &Pool<Sqlite>,
    tag_ids: &[i64],
) -> Result<Vec<i64>, (StatusCode, String)> {
    let mut normalized = tag_ids
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        return Ok(normalized);
    }
    let rows = load_tags_by_ids(pool, &normalized)
        .await
        .map_err(internal_error_tuple)?;
    if rows.len() != normalized.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            "one or more tagIds do not exist".to_string(),
        ));
    }
    Ok(normalized)
}

async fn sync_account_tag_links_with_executor(
    conn: &mut SqliteConnection,
    account_id: i64,
    tag_ids: &[i64],
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query("DELETE FROM pool_upstream_account_tags WHERE account_id = ?1")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    for tag_id in tag_ids {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_tags (
                account_id, tag_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?3)
            "#,
        )
        .bind(account_id)
        .bind(tag_id)
        .bind(&now_iso)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn sync_account_tag_links(
    pool: &Pool<Sqlite>,
    account_id: i64,
    tag_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sync_account_tag_links_with_executor(&mut *tx, account_id, tag_ids).await?;
    tx.commit().await?;
    Ok(())
}

async fn count_recent_account_conversations(
    pool: &Pool<Sqlite>,
    account_id: i64,
    lookback_hours: i64,
) -> Result<i64> {
    let lower_bound = format_utc_iso(Utc::now() - ChronoDuration::hours(lookback_hours));
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_sticky_routes
        WHERE account_id = ?1
          AND last_seen_at >= ?2
        "#,
    )
    .bind(account_id)
    .bind(lower_bound)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn load_upstream_account_groups(
    pool: &Pool<Sqlite>,
) -> Result<Vec<UpstreamAccountGroupSummary>> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ),
    >(
        r#"
        SELECT
            groups.group_name,
            notes.note,
            notes.bound_proxy_keys_json,
            notes.upstream_429_retry_enabled,
            notes.upstream_429_max_retries
        FROM (
            SELECT DISTINCT TRIM(group_name) AS group_name
            FROM pool_upstream_accounts
            WHERE group_name IS NOT NULL AND TRIM(group_name) <> ''
        ) groups
        LEFT JOIN pool_upstream_account_group_notes notes
            ON notes.group_name = groups.group_name
        ORDER BY groups.group_name COLLATE NOCASE ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                group_name,
                note,
                bound_proxy_keys_json,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
            )| {
                let upstream_429_retry_enabled = decode_group_upstream_429_retry_enabled(
                    upstream_429_retry_enabled.unwrap_or_default(),
                );
                let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
                    upstream_429_retry_enabled,
                    decode_group_upstream_429_max_retries(
                        upstream_429_max_retries.unwrap_or_default(),
                    ),
                );
                UpstreamAccountGroupSummary {
                    group_name,
                    note: normalize_optional_text(note),
                    bound_proxy_keys: decode_group_bound_proxy_keys_json(
                        bound_proxy_keys_json.as_deref(),
                    ),
                    upstream_429_retry_enabled,
                    upstream_429_max_retries,
                }
            },
        )
        .collect())
}
async fn load_upstream_account_summaries(
    pool: &Pool<Sqlite>,
) -> Result<Vec<UpstreamAccountSummary>> {
    load_upstream_account_summaries_filtered(pool, &ListUpstreamAccountsQuery::default()).await
}

async fn load_upstream_account_summaries_filtered(
    pool: &Pool<Sqlite>,
    params: &ListUpstreamAccountsQuery,
) -> Result<Vec<UpstreamAccountSummary>> {
    let legacy_status_filter =
        normalize_legacy_upstream_account_status_filter(params.status.as_deref());
    let work_status_filters = collect_normalized_upstream_account_filters(
        &params.work_status,
        legacy_status_filter.work_status,
        normalize_upstream_account_work_status_filter,
    );
    let enable_status_filters = collect_normalized_upstream_account_filters(
        &params.enable_status,
        legacy_status_filter.enable_status,
        normalize_upstream_account_enable_status_filter,
    );
    let health_status_filters = collect_normalized_upstream_account_filters(
        &params.health_status,
        legacy_status_filter.health_status,
        normalize_upstream_account_health_status_filter,
    );
    let sync_state_filter = legacy_status_filter.sync_state;
    let duplicate_info_map = load_duplicate_info_map(pool).await?;
    let mut normalized_tag_ids = params
        .tag_ids
        .iter()
        .copied()
        .filter(|tag_id| *tag_id > 0)
        .collect::<Vec<_>>();
    normalized_tag_ids.sort_unstable();
    normalized_tag_ids.dedup();
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            compact_support_status, compact_support_observed_at,
            compact_support_reason, local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        "#,
    );
    query.push(" WHERE 1 = 1");

    if params.group_ungrouped.unwrap_or(false) {
        query.push(" AND NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL");
    } else if let Some(group_search) = params
        .group_search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LOWER(TRIM(COALESCE(group_name, ''))) LIKE ")
            .push_bind(format!("%{}%", group_search.to_lowercase()));
    }

    if !normalized_tag_ids.is_empty() {
        query.push(" AND id IN (SELECT link.account_id FROM pool_upstream_account_tags link WHERE link.tag_id IN (");
        {
            let mut separated = query.separated(", ");
            for tag_id in &normalized_tag_ids {
                separated.push_bind(tag_id);
            }
        }
        query
            .push(") GROUP BY link.account_id HAVING COUNT(DISTINCT link.tag_id) = ")
            .push_bind(normalized_tag_ids.len() as i64)
            .push(")");
    }

    let rows = query
        .push(" ORDER BY updated_at DESC, id DESC")
        .build_query_as::<UpstreamAccountRow>()
        .fetch_all(pool)
        .await?;
    let now = Utc::now();
    let account_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let tag_map = load_account_tag_map(pool, &account_ids).await?;
    let active_conversation_count_map =
        load_account_active_conversation_count_map(pool, &account_ids, now.clone()).await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let latest = load_latest_usage_sample(pool, row.id).await?;
        let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
        items.push(build_summary_from_row(
            &row,
            latest.as_ref(),
            row.last_activity_at.clone(),
            tags,
            duplicate_info_map.get(&row.id).cloned(),
            active_conversation_count_map
                .get(&row.id)
                .copied()
                .unwrap_or_default(),
            now.clone(),
        ));
    }
    Ok(items
        .into_iter()
        .filter(|item| {
            matches_upstream_account_filters(
                item,
                &work_status_filters,
                &enable_status_filters,
                &health_status_filters,
                sync_state_filter,
            )
        })
        .collect())
}

async fn build_bulk_upstream_account_sync_pending_rows(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<Vec<BulkUpstreamAccountSyncRow>> {
    let mut rows = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        let display_name = load_upstream_account_row(pool, *account_id)
            .await?
            .map(|row| row.display_name)
            .unwrap_or_else(|| format!("Account {account_id}"));
        rows.push(BulkUpstreamAccountSyncRow {
            account_id: *account_id,
            display_name,
            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING.to_string(),
            detail: None,
        });
    }
    Ok(rows)
}

async fn apply_bulk_upstream_account_action(
    state: Arc<AppState>,
    account_id: i64,
    action: &str,
    group_name: Option<String>,
    tag_ids: Vec<i64>,
) -> Result<(), (StatusCode, String)> {
    let payload = match action {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE => UpdateUpstreamAccountRequest {
            display_name: None,
            group_name: None,
            group_bound_proxy_keys: None,
            note: None,
            group_note: None,
            upstream_base_url: OptionalField::Missing,
            enabled: Some(true),
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE => UpdateUpstreamAccountRequest {
            display_name: None,
            group_name: None,
            group_bound_proxy_keys: None,
            note: None,
            group_note: None,
            upstream_base_url: OptionalField::Missing,
            enabled: Some(false),
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP => UpdateUpstreamAccountRequest {
            display_name: None,
            group_name,
            group_bound_proxy_keys: None,
            note: None,
            group_note: None,
            upstream_base_url: OptionalField::Missing,
            enabled: None,
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS | BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS => {
            let current_tag_ids = load_account_tag_map(&state.pool, &[account_id])
                .await
                .map_err(internal_error_tuple)?
                .remove(&account_id)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| tag.id)
                .collect::<BTreeSet<_>>();
            let tag_id_set = tag_ids.into_iter().collect::<BTreeSet<_>>();
            let next_tag_ids = if action == BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS {
                current_tag_ids
                    .union(&tag_id_set)
                    .copied()
                    .collect::<Vec<_>>()
            } else {
                current_tag_ids
                    .difference(&tag_id_set)
                    .copied()
                    .collect::<Vec<_>>()
            };
            UpdateUpstreamAccountRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                note: None,
                group_note: None,
                upstream_base_url: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: Some(next_tag_ids),
            }
        }
        BULK_UPSTREAM_ACCOUNT_ACTION_DELETE => {
            state
                .upstream_accounts
                .account_ops
                .run_delete_account(state.clone(), account_id)
                .await?;
            return Ok(());
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "unsupported bulk action".to_string(),
            ));
        }
    };

    state
        .upstream_accounts
        .account_ops
        .run_update_account(state.clone(), account_id, payload)
        .await?;
    Ok(())
}

async fn has_ungrouped_upstream_accounts(pool: &Pool<Sqlite>) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

async fn load_upstream_account_detail(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let Some(row) = load_upstream_account_row(pool, id).await? else {
        return Ok(None);
    };
    let latest = load_latest_usage_sample(pool, row.id).await?;
    let tags = load_account_tag_map(pool, &[row.id])
        .await?
        .remove(&row.id)
        .unwrap_or_default();
    let history_rows = sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
        ORDER BY captured_at DESC
        LIMIT 128
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let mut history = history_rows
        .into_iter()
        .map(|sample| UpstreamAccountHistoryPoint {
            captured_at: sample.captured_at,
            primary_used_percent: sample.primary_used_percent,
            secondary_used_percent: sample.secondary_used_percent,
            credits_balance: sample.credits_balance,
        })
        .collect::<Vec<_>>();
    history.reverse();
    let recent_action_rows = sqlx::query_as::<_, UpstreamAccountActionEventRow>(
        r#"
        SELECT
            id, occurred_at, action, source, reason_code, reason_message,
            http_status, failure_kind, invoke_id, sticky_key, created_at
        FROM pool_upstream_account_events
        WHERE account_id = ?1
        ORDER BY occurred_at DESC, id DESC
        LIMIT 20
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let duplicate_info_map = load_duplicate_info_map(pool).await?;
    let now = Utc::now();
    let active_conversation_count =
        load_account_active_conversation_count_map(pool, &[row.id], now.clone())
            .await?
            .get(&row.id)
            .copied()
            .unwrap_or_default();
    Ok(Some(UpstreamAccountDetail {
        summary: build_summary_from_row(
            &row,
            latest.as_ref(),
            row.last_activity_at.clone(),
            tags,
            duplicate_info_map.get(&row.id).cloned(),
            active_conversation_count,
            now,
        ),
        note: row.note,
        upstream_base_url: row.upstream_base_url,
        chatgpt_user_id: row.chatgpt_user_id,
        last_refreshed_at: row.last_refreshed_at,
        history,
        recent_actions: recent_action_rows
            .iter()
            .map(build_action_event_from_row)
            .collect(),
    }))
}

async fn load_upstream_account_detail_with_actual_usage(
    state: &AppState,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let mut detail = match load_upstream_account_detail(&state.pool, id).await? {
        Some(detail) => detail,
        None => return Ok(None),
    };
    enrich_window_actual_usage_for_summaries(state, std::slice::from_mut(&mut detail.summary))
        .await?;
    Ok(Some(detail))
}

async fn load_upstream_account_row(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    let mut conn = pool.acquire().await?;
    load_upstream_account_row_conn(&mut conn, id).await
}

async fn load_upstream_account_row_conn(
    conn: &mut SqliteConnection,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            compact_support_status, compact_support_observed_at,
            compact_support_reason, local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(conn)
    .await
    .map_err(Into::into)
}

async fn load_latest_usage_sample(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<UpstreamAccountSampleRow>> {
    sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            sample.captured_at,
            sample.limit_id,
            sample.limit_name,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = sample.account_id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = sample.account_id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        FROM pool_upstream_account_limit_samples sample
        INNER JOIN pool_upstream_accounts account ON account.id = sample.account_id
        WHERE sample.account_id = ?1
        ORDER BY sample.captured_at DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn build_summary_from_row(
    row: &UpstreamAccountRow,
    sample: Option<&UpstreamAccountSampleRow>,
    last_activity_at: Option<String>,
    tags: Vec<AccountTagSummary>,
    duplicate_info: Option<DuplicateInfo>,
    active_conversation_count: i64,
    now: DateTime<Utc>,
) -> UpstreamAccountSummary {
    let local_limits = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        Some(LocalLimitSnapshot {
            primary_limit: row.local_primary_limit,
            secondary_limit: row.local_secondary_limit,
            limit_unit: row
                .local_limit_unit
                .clone()
                .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string()),
        })
    } else {
        None
    };
    let primary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_primary_limit,
            row.local_limit_unit.as_deref(),
            300,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.primary_used_percent,
                value.primary_window_minutes,
                value.primary_resets_at.as_deref(),
            )
        })
    };
    let secondary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_secondary_limit,
            row.local_limit_unit.as_deref(),
            10_080,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.secondary_used_percent,
                value.secondary_window_minutes,
                value.secondary_resets_at.as_deref(),
            )
        })
    };
    let credits = sample.and_then(|value| {
        value
            .credits_has_credits
            .map(|has_credits| CreditsSnapshot {
                has_credits: has_credits != 0,
                unlimited: value.credits_unlimited.unwrap_or_default() != 0,
                balance: value.credits_balance.clone(),
            })
    });
    let effective_routing_rule = build_effective_routing_rule(&tags);
    let status = effective_account_status(row);
    let enable_status = derive_upstream_account_enable_status(row.enabled != 0);
    let health_status = derive_upstream_account_health_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    );
    let sync_state = derive_upstream_account_sync_state(row.enabled != 0, &row.status);
    let snapshot_exhausted = persisted_usage_sample_is_exhausted(sample);
    let work_status = derive_upstream_account_work_status(
        row.enabled != 0,
        &row.status,
        health_status,
        sync_state,
        snapshot_exhausted,
        row.cooldown_until.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
        row.temporary_route_failure_streak_started_at.as_deref(),
        row.last_selected_at.as_deref(),
        now,
    );
    let display_status = classify_upstream_account_display_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    )
    .to_string();
    let compact_support = build_compact_support_state(row);

    UpstreamAccountSummary {
        id: row.id,
        kind: row.kind.clone(),
        provider: row.provider.clone(),
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        is_mother: row.is_mother != 0,
        status,
        display_status,
        enabled: row.enabled != 0,
        work_status: work_status.to_string(),
        enable_status: enable_status.to_string(),
        health_status: health_status.to_string(),
        sync_state: sync_state.to_string(),
        email: row.email.clone(),
        chatgpt_account_id: row.chatgpt_account_id.clone(),
        plan_type: resolve_effective_plan_type(
            row.plan_type.as_deref(),
            sample.and_then(|value| value.plan_type.as_deref()),
        ),
        masked_api_key: row.masked_api_key.clone(),
        last_synced_at: row.last_synced_at.clone(),
        last_successful_sync_at: row.last_successful_sync_at.clone(),
        last_activity_at: last_activity_at
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .map(format_utc_iso)
            .or(last_activity_at),
        active_conversation_count,
        last_error: row.last_error.clone(),
        last_error_at: row.last_error_at.clone(),
        last_action: row.last_action.clone(),
        last_action_source: row.last_action_source.clone(),
        last_action_reason_code: row.last_action_reason_code.clone(),
        last_action_reason_message: row.last_action_reason_message.clone(),
        last_action_http_status: row
            .last_action_http_status
            .and_then(|value| u16::try_from(value).ok()),
        last_action_invoke_id: row.last_action_invoke_id.clone(),
        last_action_at: row.last_action_at.clone(),
        token_expires_at: row.token_expires_at.clone(),
        primary_window,
        secondary_window,
        credits,
        local_limits,
        compact_support,
        duplicate_info,
        tags,
        effective_routing_rule,
    }
}

async fn enrich_window_actual_usage_for_summaries(
    state: &AppState,
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() || !sqlite_table_exists(&state.pool, "codex_invocations").await? {
        return Ok(());
    }

    let now = Utc::now();
    let Some((plans, query_start, query_end)) = collect_account_window_usage_plans(items, now)
    else {
        return Ok(());
    };
    let account_ids = plans.keys().copied().collect::<Vec<_>>();
    if account_ids.is_empty() {
        return Ok(());
    }

    let query_start_at = format_naive(query_start.with_timezone(&Shanghai).naive_local());
    let query_end_at = format_naive(query_end.with_timezone(&Shanghai).naive_local());
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let mut rows = Vec::new();

    let live_start = query_start.max(retention_cutoff);
    if live_start <= query_end {
        let live_start_at = format_naive(live_start.with_timezone(&Shanghai).naive_local());
        rows.extend(
            load_window_actual_usage_rows_from_pool(
                &state.pool,
                &account_ids,
                &live_start_at,
                &query_end_at,
                None,
            )
            .await?,
        );
    }

    if query_start < retention_cutoff {
        let archive_end = query_end.min(retention_cutoff - ChronoDuration::seconds(1));
        if query_start <= archive_end {
            let archive_end_at = format_naive(archive_end.with_timezone(&Shanghai).naive_local());
            rows.extend(
                load_window_actual_usage_rows_from_archives(
                    &state.pool,
                    &account_ids,
                    &query_start_at,
                    &archive_end_at,
                    &state.config.archive_dir,
                )
                .await?,
            );
        }
    }

    let usage = fold_account_window_usage_rows(rows, &plans);
    apply_window_actual_usage_to_summaries(items, &usage);
    Ok(())
}

fn collect_account_window_usage_plans(
    items: &[UpstreamAccountSummary],
    now: DateTime<Utc>,
) -> Option<(
    HashMap<i64, AccountWindowUsagePlan>,
    DateTime<Utc>,
    DateTime<Utc>,
)> {
    let mut plans = HashMap::new();
    let mut earliest_start_at: Option<DateTime<Utc>> = None;
    let mut latest_end_at: Option<DateTime<Utc>> = None;

    for item in items {
        let primary = item.primary_window.as_ref().and_then(|window| {
            build_window_usage_range(
                now,
                window.window_duration_mins,
                window.resets_at.as_deref(),
            )
        });
        let secondary = item.secondary_window.as_ref().and_then(|window| {
            build_window_usage_range(
                now,
                window.window_duration_mins,
                window.resets_at.as_deref(),
            )
        });
        if primary.is_none() && secondary.is_none() {
            continue;
        }

        for range in [primary, secondary].into_iter().flatten() {
            earliest_start_at = Some(
                earliest_start_at
                    .map(|value| value.min(range.start_at))
                    .unwrap_or(range.start_at),
            );
            latest_end_at = Some(
                latest_end_at
                    .map(|value| value.max(range.end_at))
                    .unwrap_or(range.end_at),
            );
        }

        plans.insert(
            item.id,
            AccountWindowUsagePlan {
                primary: primary.map(AccountWindowUsageRangeBounds::into_range),
                secondary: secondary.map(AccountWindowUsageRangeBounds::into_range),
            },
        );
    }

    Some((plans, earliest_start_at?, latest_end_at?))
}

fn build_window_usage_range(
    now: DateTime<Utc>,
    window_duration_mins: i64,
    resets_at: Option<&str>,
) -> Option<AccountWindowUsageRangeBounds> {
    if window_duration_mins <= 0 {
        return None;
    }
    let window_anchor = resets_at.and_then(parse_rfc3339_utc).unwrap_or(now);
    Some(AccountWindowUsageRangeBounds {
        start_at: window_anchor - ChronoDuration::minutes(window_duration_mins),
        end_at: window_anchor.min(now),
    })
}

async fn load_window_actual_usage_rows_from_pool(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_at: &str,
    end_at: &str,
    end_before: Option<&str>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }

    let upstream_account_id_sql = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            occurred_at,
        "#,
    );
    query
        .push(upstream_account_id_sql)
        .push(
            r#"
            AS upstream_account_id,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost
        FROM codex_invocations
        WHERE occurred_at >=
        "#,
        )
        .push_bind(start_at)
        .push(" AND occurred_at <= ")
        .push_bind(end_at)
        .push(" AND ")
        .push(upstream_account_id_sql)
        .push(" IS NOT NULL");

    if let Some(end_before) = end_before {
        query.push(" AND occurred_at < ").push_bind(end_before);
    }

    query
        .push(" AND ")
        .push(upstream_account_id_sql)
        .push(" IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") ORDER BY occurred_at ASC");

    query
        .build_query_as::<AccountWindowUsageRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn load_window_actual_usage_rows_from_archives(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_at: &str,
    end_at: &str,
    archive_dir: &Path,
) -> Result<Vec<AccountWindowUsageRow>> {
    if account_ids.is_empty() || !sqlite_table_exists(pool, "archive_batches").await? {
        return Ok(Vec::new());
    }

    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND (coverage_end_at IS NULL OR coverage_end_at >= ?2)
          AND (coverage_start_at IS NULL OR coverage_start_at <= ?3)
        ORDER BY month_key DESC, day_key DESC, part_key DESC, created_at DESC, id DESC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(start_at)
    .bind(end_at)
    .fetch_all(pool)
    .await?;

    let mut rows = Vec::new();
    for archive_file in archive_files {
        let archive_path = resolve_archive_batch_path(archive_dir, &archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                file_path = %archive_path.display(),
                "skipping missing invocation archive batch while calculating account window usage"
            );
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        rows.extend(
            load_window_actual_usage_rows_from_pool(
                &archive_pool,
                account_ids,
                start_at,
                end_at,
                None,
            )
            .await?,
        );
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(rows)
}

fn resolve_archive_batch_path(archive_dir: &Path, file_path: &str) -> PathBuf {
    let path = PathBuf::from(file_path);
    if path.is_absolute() {
        path
    } else {
        archive_dir.join(path)
    }
}

fn fold_account_window_usage_rows(
    rows: Vec<AccountWindowUsageRow>,
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> HashMap<i64, AccountWindowUsageSummary> {
    let mut usage = plans
        .keys()
        .copied()
        .map(|account_id| (account_id, AccountWindowUsageSummary::default()))
        .collect::<HashMap<_, _>>();

    for row in rows {
        let Some(plan) = plans.get(&row.upstream_account_id) else {
            continue;
        };
        let entry = usage.entry(row.upstream_account_id).or_default();
        if plan.primary.as_ref().is_some_and(|range| {
            row.occurred_at.as_str() >= range.start_at.as_str()
                && row.occurred_at.as_str() <= range.end_at.as_str()
        }) {
            entry.primary.add_row(&row);
        }
        if plan.secondary.as_ref().is_some_and(|range| {
            row.occurred_at.as_str() >= range.start_at.as_str()
                && row.occurred_at.as_str() <= range.end_at.as_str()
        }) {
            entry.secondary.add_row(&row);
        }
    }

    usage
}

fn apply_window_actual_usage_to_summaries(
    items: &mut [UpstreamAccountSummary],
    usage: &HashMap<i64, AccountWindowUsageSummary>,
) {
    for item in items {
        let account_usage = usage.get(&item.id).copied().unwrap_or_default();
        if let Some(window) = item.primary_window.as_mut() {
            window.actual_usage = Some(account_usage.primary.into_snapshot());
        }
        if let Some(window) = item.secondary_window.as_mut() {
            window.actual_usage = Some(account_usage.secondary.into_snapshot());
        }
    }
}

async fn load_account_active_conversation_count_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    now: DateTime<Utc>,
) -> Result<HashMap<i64, i64>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let active_cutoff =
        format_utc_iso(now - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES));
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account_id,
            COUNT(*) AS active_conversation_count
        FROM pool_sticky_routes
        WHERE last_seen_at >=
        "#,
    );
    query.push_bind(&active_cutoff).push(" AND account_id IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(") GROUP BY account_id")
        .build_query_as::<AccountActiveConversationCountRow>()
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.account_id, row.active_conversation_count))
        .collect())
}

fn build_compact_support_state(row: &UpstreamAccountRow) -> CompactSupportState {
    let status = row
        .compact_support_status
        .as_deref()
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                COMPACT_SUPPORT_STATUS_UNKNOWN
                    | COMPACT_SUPPORT_STATUS_SUPPORTED
                    | COMPACT_SUPPORT_STATUS_UNSUPPORTED
            )
        })
        .unwrap_or(COMPACT_SUPPORT_STATUS_UNKNOWN)
        .to_string();
    CompactSupportState {
        status,
        observed_at: row.compact_support_observed_at.clone(),
        reason: row.compact_support_reason.clone(),
    }
}

fn build_action_event_from_row(row: &UpstreamAccountActionEventRow) -> UpstreamAccountActionEvent {
    UpstreamAccountActionEvent {
        id: row.id,
        occurred_at: row.occurred_at.clone(),
        action: row.action.clone(),
        source: row.source.clone(),
        reason_code: row.reason_code.clone(),
        reason_message: row.reason_message.clone(),
        http_status: row.http_status.and_then(|value| u16::try_from(value).ok()),
        failure_kind: row.failure_kind.clone(),
        invoke_id: row.invoke_id.clone(),
        sticky_key: row.sticky_key.clone(),
        created_at: row.created_at.clone(),
    }
}

pub(crate) async fn load_account_last_activity_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, String>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id AS account_id, last_activity_at FROM pool_upstream_accounts WHERE last_activity_at IS NOT NULL AND id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(")");

    let rows = query
        .build_query_as::<AccountLastActivityRow>()
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.account_id, row.last_activity_at))
        .collect())
}

pub(crate) async fn backfill_upstream_account_last_activity_from_live_invocations(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    if !sqlite_table_exists(pool, "codex_invocations")
        .await
        .context("failed to inspect codex_invocations existence")?
    {
        return Ok(0);
    }

    let updated = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_activity_at = (
                SELECT MAX(occurred_at)
                FROM codex_invocations
                WHERE CASE
                    WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER)
                END = pool_upstream_accounts.id
            ),
            last_activity_live_backfill_completed = 1
        WHERE last_activity_at IS NULL
          AND last_activity_live_backfill_completed = 0
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill pool_upstream_accounts.last_activity_at from live invocations")?;
    Ok(updated.rows_affected())
}

async fn group_has_accounts(pool: &Pool<Sqlite>, group_name: &str) -> Result<bool> {
    let mut conn = pool.acquire().await?;
    group_has_accounts_conn(&mut conn, group_name).await
}

async fn group_account_count_conn(conn: &mut SqliteConnection, group_name: &str) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .fetch_one(conn)
    .await
    .map_err(Into::into)
}

async fn group_has_accounts_conn(conn: &mut SqliteConnection, group_name: &str) -> Result<bool> {
    Ok(group_account_count_conn(conn, group_name).await? > 0)
}

fn normalize_bound_proxy_keys(bound_proxy_keys: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    bound_proxy_keys
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn decode_group_bound_proxy_keys_json(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .map(normalize_bound_proxy_keys)
        .unwrap_or_default()
}

fn decode_group_upstream_429_retry_enabled(raw: i64) -> bool {
    raw != 0
}

fn normalize_group_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

fn normalize_enabled_group_upstream_429_max_retries(value: u8) -> u8 {
    normalize_group_upstream_429_max_retries(value).max(1)
}

fn normalize_group_upstream_429_retry_metadata(
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
) -> u8 {
    if upstream_429_retry_enabled {
        normalize_enabled_group_upstream_429_max_retries(upstream_429_max_retries)
    } else {
        0
    }
}

fn decode_group_upstream_429_max_retries(raw: i64) -> u8 {
    normalize_group_upstream_429_max_retries(raw.max(0) as u8)
}

fn encode_group_bound_proxy_keys_json(bound_proxy_keys: &[String]) -> Result<String> {
    serde_json::to_string(bound_proxy_keys).context("failed to encode group bound proxy keys")
}

fn missing_request_group_error_message() -> String {
    "groupName is required for upstream accounts".to_string()
}

fn missing_account_group_error_message() -> String {
    "upstream account is not assigned to a group; assign it to a group with at least one bound forward proxy node".to_string()
}

fn missing_group_bound_proxy_error_message(group_name: &str) -> String {
    format!(
        "upstream account group \"{group_name}\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    )
}

fn missing_selectable_group_bound_proxy_error_message(group_name: &str) -> String {
    format!("upstream account group \"{group_name}\" has no selectable bound forward proxy nodes")
}

fn build_requested_group_metadata_changes(
    note: Option<String>,
    note_was_requested: bool,
    bound_proxy_keys: Option<Vec<String>>,
    bound_proxy_keys_was_requested: bool,
) -> RequestedGroupMetadataChanges {
    RequestedGroupMetadataChanges {
        note: normalize_optional_text(note),
        note_was_requested,
        bound_proxy_keys: if bound_proxy_keys_was_requested {
            normalize_bound_proxy_keys(bound_proxy_keys.unwrap_or_default())
        } else {
            Vec::new()
        },
        bound_proxy_keys_was_requested,
    }
}

pub(crate) fn required_account_forward_proxy_scope(
    group_name: Option<&str>,
    bound_proxy_keys: Vec<String>,
) -> Result<ForwardProxyRouteScope> {
    let normalized_group_name = group_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let normalized_bound_proxy_keys = normalize_bound_proxy_keys(bound_proxy_keys);
    if normalized_bound_proxy_keys.is_empty() {
        bail!(missing_group_bound_proxy_error_message(
            &normalized_group_name
        ));
    }
    Ok(ForwardProxyRouteScope::BoundGroup {
        group_name: normalized_group_name,
        bound_proxy_keys: normalized_bound_proxy_keys,
    })
}

fn map_required_group_proxy_selection_error(
    scope: &ForwardProxyRouteScope,
    err: anyhow::Error,
) -> anyhow::Error {
    match scope {
        ForwardProxyRouteScope::BoundGroup { group_name, .. }
            if err
                .to_string()
                .contains("bound forward proxy group has no selectable nodes") =>
        {
            anyhow!(missing_selectable_group_bound_proxy_error_message(
                group_name
            ))
        }
        _ => err,
    }
}

async fn ensure_required_group_proxy_scope_selectable(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<()> {
    select_forward_proxy_for_scope(state, scope)
        .await
        .map(|_| ())
        .map_err(|err| map_required_group_proxy_selection_error(scope, err))
}

async fn resolve_required_group_proxy_binding_for_write(
    state: &AppState,
    group_name: Option<String>,
    requested_bound_proxy_keys: Option<Vec<String>>,
) -> Result<ResolvedRequiredGroupProxyBinding, (StatusCode, String)> {
    let group_name = normalize_optional_text(group_name).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            missing_request_group_error_message(),
        )
    })?;
    let bound_proxy_keys = if let Some(requested_bound_proxy_keys) = requested_bound_proxy_keys {
        normalize_bound_proxy_keys(requested_bound_proxy_keys)
    } else {
        load_group_metadata(&state.pool, Some(&group_name))
            .await
            .map_err(internal_error_tuple)?
            .bound_proxy_keys
    };
    let scope = required_account_forward_proxy_scope(Some(&group_name), bound_proxy_keys.clone())
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    ensure_required_group_proxy_scope_selectable(state, &scope)
        .await
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(ResolvedRequiredGroupProxyBinding {
        group_name,
        bound_proxy_keys,
    })
}

async fn load_group_metadata_conn(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: &str,
) -> Result<Option<UpstreamAccountGroupMetadata>> {
    sqlx::query_as::<_, (String, Option<String>, i64, i64)>(
        r#"
        SELECT
            note,
            bound_proxy_keys_json,
            upstream_429_retry_enabled,
            upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .fetch_optional(executor)
    .await
    .map(|row| {
        row.map(
            |(
                note,
                bound_proxy_keys_json,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
            )| {
                let upstream_429_retry_enabled =
                    decode_group_upstream_429_retry_enabled(upstream_429_retry_enabled);
                let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
                    upstream_429_retry_enabled,
                    decode_group_upstream_429_max_retries(upstream_429_max_retries),
                );
                UpstreamAccountGroupMetadata {
                    note: normalize_optional_text(Some(note)),
                    bound_proxy_keys: decode_group_bound_proxy_keys_json(
                        bound_proxy_keys_json.as_deref(),
                    ),
                    upstream_429_retry_enabled,
                    upstream_429_max_retries,
                }
            },
        )
    })
    .map_err(Into::into)
}

async fn load_group_metadata(
    pool: &Pool<Sqlite>,
    group_name: Option<&str>,
) -> Result<UpstreamAccountGroupMetadata> {
    let Some(group_name) = group_name else {
        return Ok(UpstreamAccountGroupMetadata::default());
    };
    let mut conn = pool.acquire().await?;
    Ok(load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default())
}

pub(crate) async fn load_required_account_forward_proxy_scope_from_group_metadata(
    pool: &Pool<Sqlite>,
    group_name: Option<&str>,
) -> Result<ForwardProxyRouteScope> {
    let bound_proxy_keys = load_group_metadata(pool, group_name)
        .await?
        .bound_proxy_keys;
    required_account_forward_proxy_scope(group_name, bound_proxy_keys)
}

async fn save_group_metadata_record_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
    metadata: UpstreamAccountGroupMetadata,
) -> Result<()> {
    let normalized_note = normalize_optional_text(metadata.note);
    let normalized_bound_proxy_keys = normalize_bound_proxy_keys(metadata.bound_proxy_keys);
    let normalized_upstream_429_retry_enabled = metadata.upstream_429_retry_enabled;
    let normalized_upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        normalized_upstream_429_retry_enabled,
        metadata.upstream_429_max_retries,
    );
    if normalized_note.is_none()
        && normalized_bound_proxy_keys.is_empty()
        && !normalized_upstream_429_retry_enabled
        && normalized_upstream_429_max_retries == 0
    {
        sqlx::query(
            r#"
            DELETE FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
        )
        .bind(group_name)
        .execute(conn)
        .await?;
        return Ok(());
    }

    let now_iso = format_utc_iso(Utc::now());
    let bound_proxy_keys_json = encode_group_bound_proxy_keys_json(&normalized_bound_proxy_keys)?;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name,
            note,
            bound_proxy_keys_json,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
        ON CONFLICT(group_name) DO UPDATE SET
            note = excluded.note,
            bound_proxy_keys_json = excluded.bound_proxy_keys_json,
            upstream_429_retry_enabled = excluded.upstream_429_retry_enabled,
            upstream_429_max_retries = excluded.upstream_429_max_retries,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(group_name)
    .bind(normalized_note.unwrap_or_default())
    .bind(bound_proxy_keys_json)
    .bind(if normalized_upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(normalized_upstream_429_max_retries))
    .bind(now_iso)
    .execute(conn)
    .await?;
    Ok(())
}

#[allow(dead_code)]
async fn save_group_note_record(
    pool: &Pool<Sqlite>,
    group_name: &str,
    note: Option<String>,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    save_group_note_record_conn(&mut conn, group_name, note).await
}

async fn save_group_note_record_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
    note: Option<String>,
) -> Result<()> {
    let mut metadata = load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default();
    metadata.note = note;
    save_group_metadata_record_conn(conn, group_name, metadata).await
}

async fn save_group_metadata_for_single_account_group(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    changes: &RequestedGroupMetadataChanges,
) -> Result<()> {
    if !changes.was_requested() {
        return Ok(());
    }
    let Some(group_name) = group_name else {
        return Ok(());
    };
    if group_account_count_conn(conn, group_name).await? != 1 {
        return Ok(());
    }
    let mut metadata = load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default();
    if changes.note_was_requested {
        metadata.note = changes.note.clone();
    }
    if changes.bound_proxy_keys_was_requested {
        metadata.bound_proxy_keys = changes.bound_proxy_keys.clone();
    }
    save_group_metadata_record_conn(conn, group_name, metadata).await
}

async fn save_group_metadata_after_account_write(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    changes: &RequestedGroupMetadataChanges,
    target_group_already_had_current_account: bool,
) -> Result<()> {
    if target_group_already_had_current_account {
        return Ok(());
    }
    save_group_metadata_for_single_account_group(conn, group_name, changes).await
}

async fn cleanup_orphaned_group_metadata(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
) -> Result<()> {
    let Some(group_name) = group_name else {
        return Ok(());
    };
    if group_has_accounts_conn(conn, group_name).await? {
        return Ok(());
    }
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .execute(conn)
    .await?;
    Ok(())
}

async fn load_login_session_by_login_id_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, group_name, group_bound_proxy_keys_json, is_mother, note, tag_ids_json, group_note,
            mailbox_session_id, generated_mailbox_address AS mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE login_id = ?1
        LIMIT 1
        "#,
    )
    .bind(login_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

async fn load_login_session_by_login_id(
    pool: &Pool<Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    load_login_session_by_login_id_with_executor(pool, login_id).await
}

async fn load_login_session_by_state(
    pool: &Pool<Sqlite>,
    state_value: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, group_name, group_bound_proxy_keys_json, is_mother, note, tag_ids_json, group_note,
            mailbox_session_id, generated_mailbox_address AS mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE state = ?1
        LIMIT 1
        "#,
    )
    .bind(state_value)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn expire_pending_login_sessions(pool: &Pool<Sqlite>) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?1, updated_at = ?2
        WHERE status = ?3 AND expires_at < ?2
        "#,
    )
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(())
}

async fn load_oauth_mailbox_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Option<OauthMailboxSessionRow>> {
    sqlx::query_as::<_, OauthMailboxSessionRow>(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE session_id = ?1
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_oauth_mailbox_sessions(
    pool: &Pool<Sqlite>,
    session_ids: &[String],
) -> Result<Vec<OauthMailboxSessionRow>> {
    if session_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE session_id IN (
        "#,
    );
    let mut separated = builder.separated(", ");
    for session_id in session_ids {
        separated.push_bind(session_id);
    }
    separated.push_unseparated(")");
    builder.push(" ORDER BY created_at ASC");
    builder
        .build_query_as::<OauthMailboxSessionRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn delete_oauth_mailbox_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    session_id: &str,
) -> Result<u64> {
    let affected = sqlx::query(
        r#"
        DELETE FROM pool_oauth_mailbox_sessions
        WHERE session_id = ?1
        "#,
    )
    .bind(session_id)
    .execute(executor)
    .await?
    .rows_affected();
    Ok(affected)
}

async fn cleanup_expired_oauth_mailbox_sessions(state: &AppState) -> Result<()> {
    let moemail_config = state.config.upstream_accounts_moemail.as_ref();
    let now_iso = format_utc_iso(Utc::now());
    let expired_rows = sqlx::query_as::<_, OauthMailboxSessionRow>(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE expires_at <= ?1
        ORDER BY expires_at ASC
        "#,
    )
    .bind(&now_iso)
    .fetch_all(&state.pool)
    .await?;

    for row in expired_rows {
        if expired_mailbox_session_requires_remote_delete(&row)
            && let Some(config) = moemail_config
            && let Err(err) =
                moemail_delete_email(&state.http_clients.shared, config, &row.remote_email_id).await
        {
            debug!(
                mailbox_session_id = %row.session_id,
                remote_email_id = %row.remote_email_id,
                error = %err,
                "failed to delete expired moemail mailbox"
            );
        }
        delete_oauth_mailbox_session_with_executor(&state.pool, &row.session_id).await?;
    }
    Ok(())
}

async fn complete_login_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
    account_id: i64,
    group_note_snapshot: Option<String>,
    previous_updated_at: &str,
    preserve_pending_updated_at: bool,
) -> Result<()> {
    let consumed_at = next_login_session_updated_at(Some(previous_updated_at));
    let completed_updated_at = if preserve_pending_updated_at {
        previous_updated_at.to_string()
    } else {
        consumed_at.clone()
    };
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            account_id = ?3,
            group_note = ?4,
            updated_at = ?5,
            consumed_at = ?6
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_COMPLETED)
    .bind(account_id)
    .bind(group_note_snapshot)
    .bind(&completed_updated_at)
    .bind(&consumed_at)
    .execute(executor)
    .await?;
    Ok(())
}

async fn load_group_note_snapshot_conn(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: Option<&str>,
    fallback_note: Option<&str>,
) -> Result<Option<String>> {
    let Some(group_name) = group_name else {
        return Ok(None);
    };
    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT note
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .fetch_optional(executor)
    .await?
    .flatten();
    Ok(normalize_optional_text(group_note).or_else(|| fallback_note.map(str::to_string)))
}

fn next_login_session_updated_at(previous_updated_at: Option<&str>) -> String {
    let mut next_updated_at =
        parse_rfc3339_utc(&Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(Utc::now);
    if let Some(previous_updated_at) = previous_updated_at
        && let Some(previous_updated_at) = parse_rfc3339_utc(previous_updated_at)
        && next_updated_at <= previous_updated_at
    {
        next_updated_at = previous_updated_at + ChronoDuration::milliseconds(1);
    }
    next_updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn fail_login_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            error_message = ?3,
            consumed_at = ?4,
            updated_at = ?4
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_FAILED)
    .bind(error_message)
    .bind(&now_iso)
    .execute(executor)
    .await?;
    Ok(())
}

async fn fail_login_session(
    pool: &Pool<Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    fail_login_session_with_executor(pool, login_id, error_message).await
}

async fn mark_login_session_expired(pool: &Pool<Sqlite>, login_id: &str) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            updated_at = ?3
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

fn login_session_to_response(row: &OauthLoginSessionRow) -> LoginSessionStatusResponse {
    LoginSessionStatusResponse {
        login_id: row.login_id.clone(),
        status: row.status.clone(),
        auth_url: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.auth_url.clone())
        } else {
            None
        },
        redirect_uri: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.redirect_uri.clone())
        } else {
            None
        },
        expires_at: row.expires_at.clone(),
        updated_at: row.updated_at.clone(),
        account_id: row.account_id,
        error: row.error_message.clone(),
        sync_applied: None,
    }
}

fn login_session_to_response_with_sync_applied(
    row: &OauthLoginSessionRow,
    sync_applied: bool,
) -> LoginSessionStatusResponse {
    let mut response = login_session_to_response(row);
    response.sync_applied = Some(sync_applied);
    response
}

fn login_session_required_forward_proxy_scope(
    row: &OauthLoginSessionRow,
) -> Result<ForwardProxyRouteScope> {
    required_account_forward_proxy_scope(
        row.group_name.as_deref(),
        decode_group_bound_proxy_keys_json(row.group_bound_proxy_keys_json.as_deref()),
    )
}

#[derive(Debug, Clone)]
struct ParsedMailboxCode {
    value: String,
    source: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct ParsedMailboxInvite {
    subject: String,
    copy_value: String,
    copy_label: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailConfigPayload {
    email_domains: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailGenerateEmailPayload {
    id: String,
    email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailEmailListPayload {
    emails: Vec<MoeMailEmailSummary>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailEmailSummary {
    id: String,
    address: String,
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageListPayload {
    messages: Vec<MoeMailMessageSummary>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageSummary {
    id: String,
    subject: Option<String>,
    received_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageDetailPayload {
    message: MoeMailMessageDetail,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageDetail {
    id: String,
    subject: Option<String>,
    content: Option<String>,
    html: Option<String>,
    received_at: Option<String>,
}

const MAILBOX_CODE_CONTEXT_WINDOW_BYTES: usize = 64;
const OAUTH_BRAND_MARKERS: &[&str] = &["openai", "chatgpt"];
const OAUTH_STRONG_CODE_MARKERS: &[&str] = &[
    "verification code",
    "temporary verification code",
    "one-time code",
    "one time code",
    "security code",
    "验证码",
    "驗證碼",
    "校验码",
    "校驗碼",
    "验证代码",
    "驗證代碼",
    "認證碼",
    "認証コード",
    "인증 코드",
    "인증번호",
];
const OAUTH_WEAK_CODE_MARKERS: &[&str] = &[
    "your code",
    "code is",
    "code:",
    "temporary code",
    "代码为",
    "代碼為",
    "代码是",
    "代碼是",
    "臨時代碼",
    "临时代码",
];
const OAUTH_INVITE_SUBJECT_MARKERS: &[&str] = &[
    "has invited you",
    "invited you to",
    "invite you to",
    "邀请你",
    "邀請你",
    "邀请您",
    "邀請您",
    "招待",
    "초대",
];
const OAUTH_INVITE_BODY_MARKERS: &[&str] = &[
    "join workspace",
    "join the workspace",
    "accept invitation",
    "accept invite",
    "workspace invite",
    "accept the invitation",
    "加入工作区",
    "加入工作區",
    "加入工作空间",
    "加入工作空間",
    "接受邀请",
    "接受邀請",
    "接受此邀请",
    "接受此邀請",
    "ワークスペース",
    "招待",
    "워크스페이스",
    "초대 수락",
];
static OAUTH_CODE_CANDIDATE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:^|[^0-9])([0-9]{4,8})(?:[^0-9]|$)").expect("valid oauth code candidate regex")
});
static URL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"https?://[^\s"'<>)]+"#).expect("valid url regex"));
static HTML_TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<[^>]+>").expect("valid html tag regex"));
static BASIC_EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$")
        .expect("valid basic email regex")
});

fn oauth_mailbox_status_from_row(row: &OauthMailboxSessionRow) -> OauthMailboxStatus {
    OauthMailboxStatus {
        session_id: row.session_id.clone(),
        email_address: row.email_address.clone(),
        expires_at: row.expires_at.clone(),
        latest_code: match (
            row.latest_code_value.clone(),
            row.latest_code_source.clone(),
            row.latest_code_updated_at.clone(),
        ) {
            (Some(value), Some(source), Some(updated_at)) => Some(OauthMailboxCodeSummary {
                value,
                source,
                updated_at,
            }),
            _ => None,
        },
        invite: match (
            row.invite_subject.clone(),
            row.invite_copy_value.clone(),
            row.invite_copy_label.clone(),
            row.invite_updated_at.clone(),
        ) {
            (Some(subject), Some(copy_value), Some(copy_label), Some(updated_at)) => {
                Some(OauthInviteSummary {
                    subject,
                    copy_value,
                    copy_label,
                    updated_at,
                })
            }
            _ => None,
        },
        invited: row.invited != 0,
        error: None,
    }
}

fn oauth_mailbox_session_supported_response(
    session_id: String,
    email_address: String,
    expires_at: String,
    source: &str,
) -> OauthMailboxSessionResponse {
    OauthMailboxSessionResponse {
        email_address,
        supported: true,
        session_id: Some(session_id),
        expires_at: Some(expires_at),
        source: Some(source.to_string()),
        reason: None,
    }
}

fn oauth_mailbox_session_unsupported_response(
    email_address: String,
    reason: &str,
) -> OauthMailboxSessionResponse {
    OauthMailboxSessionResponse {
        email_address,
        supported: false,
        session_id: None,
        expires_at: None,
        source: None,
        reason: Some(reason.to_string()),
    }
}

fn normalize_mailbox_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn normalize_mailbox_domain(value: &str) -> Option<String> {
    let trimmed = value
        .trim()
        .trim_matches(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'');
    if trimmed.is_empty() {
        return None;
    }
    let without_prefix = trimmed.trim_start_matches('@');
    let domain_like = without_prefix
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or(without_prefix)
        .trim()
        .trim_start_matches('@')
        .trim_end_matches('.');
    if domain_like.is_empty() {
        return None;
    }
    Some(domain_like.to_ascii_lowercase())
}

fn moemail_supported_domains(payload: &MoeMailConfigPayload) -> HashSet<String> {
    payload
        .email_domains
        .as_deref()
        .unwrap_or_default()
        .split(|ch: char| matches!(ch, ',' | ';' | '\n' | '\r'))
        .filter_map(normalize_mailbox_domain)
        .collect()
}

fn mailbox_local_part(value: &str) -> Option<&str> {
    let (local_part, domain) = value.split_once('@')?;
    if local_part.is_empty() || domain.is_empty() {
        return None;
    }
    Some(local_part)
}

#[derive(Debug, PartialEq, Eq)]
enum RequestedManualMailboxAddress {
    Missing,
    Valid(String),
    Invalid(String),
}

fn requested_manual_mailbox_address(
    raw_email_address: Option<&str>,
) -> RequestedManualMailboxAddress {
    match raw_email_address {
        None => RequestedManualMailboxAddress::Missing,
        Some(value) => match normalize_mailbox_address(value) {
            Some(normalized) => RequestedManualMailboxAddress::Valid(normalized),
            None => RequestedManualMailboxAddress::Invalid(value.to_string()),
        },
    }
}

fn mailbox_address_is_valid(value: &str) -> bool {
    BASIC_EMAIL_REGEX.is_match(value.trim())
}

fn upstream_mailbox_config(
    config: &AppConfig,
) -> Result<&UpstreamAccountsMoeMailConfig, (StatusCode, String)> {
    config.upstream_accounts_moemail.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "oauth temp mail requires {}, {}, and {}",
                ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL,
                ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY,
                ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN
            ),
        )
    })
}

fn validate_mailbox_binding_fields(
    mailbox_session_id: Option<&str>,
    mailbox_address: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    match (mailbox_session_id, mailbox_address) {
        (Some(_), Some(_)) | (None, None) => Ok(()),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "mailboxSessionId and mailboxAddress must be provided together".to_string(),
        )),
    }
}

fn mailbox_addresses_match(left: Option<&str>, right: Option<&str>) -> bool {
    normalize_mailbox_address(left.unwrap_or_default())
        == normalize_mailbox_address(right.unwrap_or_default())
}

fn expired_mailbox_session_requires_remote_delete(row: &OauthMailboxSessionRow) -> bool {
    row.mailbox_source.as_deref() != Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
}

fn normalize_mailbox_session_expires_at(value: Option<&str>, fallback: DateTime<Utc>) -> String {
    value
        .and_then(|raw| {
            DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|parsed| format_utc_iso(parsed.with_timezone(&Utc)))
        })
        .unwrap_or_else(|| format_utc_iso(fallback))
}

async fn validate_mailbox_binding(
    pool: &Pool<Sqlite>,
    mailbox_session_id: Option<&str>,
    mailbox_address: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    validate_mailbox_binding_fields(mailbox_session_id, mailbox_address)?;
    let Some(session_id) = mailbox_session_id else {
        return Ok(());
    };
    let Some(expected_address) = mailbox_address else {
        return Ok(());
    };
    let row = load_oauth_mailbox_session(pool, session_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "mailbox session is missing or expired".to_string(),
            )
        })?;
    if normalize_mailbox_address(&row.email_address) != normalize_mailbox_address(expected_address)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "mailboxAddress no longer matches the mailbox session".to_string(),
        ));
    }
    Ok(())
}

fn strip_html_tags(raw: &str) -> String {
    HTML_TAG_REGEX.replace_all(raw, " ").into_owned()
}

fn normalize_mailbox_text(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut previous_was_space = true;

    for ch in raw.chars() {
        let mapped = match ch {
            '\u{00a0}' | '\u{3000}' => ' ',
            '０'..='９' => {
                char::from_u32(u32::from(ch) - u32::from('０') + u32::from('0')).unwrap_or(ch)
            }
            'Ａ'..='Ｚ' => {
                char::from_u32(u32::from(ch) - u32::from('Ａ') + u32::from('a')).unwrap_or(ch)
            }
            'ａ'..='ｚ' => {
                char::from_u32(u32::from(ch) - u32::from('ａ') + u32::from('a')).unwrap_or(ch)
            }
            '：' => ':',
            '－' => '-',
            '／' => '/',
            '．' => '.',
            '，' => ',',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            _ if ch.is_ascii_uppercase() => ch.to_ascii_lowercase(),
            _ => ch,
        };

        if mapped.is_whitespace() {
            if !previous_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(mapped);
            previous_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn mailbox_text_contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn mailbox_text_has_brand(text: &str) -> bool {
    mailbox_text_contains_any(text, OAUTH_BRAND_MARKERS)
}

fn clamp_mailbox_context_start(raw: &str, index: usize) -> usize {
    let mut candidate = index.min(raw.len());
    while candidate > 0 && !raw.is_char_boundary(candidate) {
        candidate -= 1;
    }
    candidate
}

fn clamp_mailbox_context_end(raw: &str, index: usize) -> usize {
    let mut candidate = index.min(raw.len());
    while candidate < raw.len() && !raw.is_char_boundary(candidate) {
        candidate += 1;
    }
    candidate
}

fn mailbox_context_slice(raw: &str, start: usize, end: usize, radius: usize) -> &str {
    let context_start = clamp_mailbox_context_start(raw, start.saturating_sub(radius));
    let context_end = clamp_mailbox_context_end(raw, end.saturating_add(radius));
    &raw[context_start..context_end]
}

fn mailbox_context_before(raw: &str, index: usize, radius: usize) -> &str {
    let context_start = clamp_mailbox_context_start(raw, index.saturating_sub(radius));
    let context_end = clamp_mailbox_context_end(raw, index);
    &raw[context_start..context_end]
}

fn extract_mailbox_code_candidate(text: &str, message_has_brand: bool) -> Option<String> {
    let mut best_match: Option<(u8, usize, String)> = None;

    for captures in OAUTH_CODE_CANDIDATE_REGEX.captures_iter(text) {
        let whole_match = captures.get(0)?;
        let digit_match = captures.get(1)?;
        let context = mailbox_context_slice(
            text,
            whole_match.start(),
            whole_match.end(),
            MAILBOX_CODE_CONTEXT_WINDOW_BYTES,
        );
        let prefix_context =
            mailbox_context_before(text, digit_match.start(), MAILBOX_CODE_CONTEXT_WINDOW_BYTES);
        let context_has_strong_code =
            mailbox_text_contains_any(prefix_context, OAUTH_STRONG_CODE_MARKERS);
        let context_has_weak_code =
            mailbox_text_contains_any(prefix_context, OAUTH_WEAK_CODE_MARKERS);
        let context_has_brand =
            mailbox_text_has_brand(prefix_context) || mailbox_text_has_brand(context);
        let score = if context_has_strong_code {
            3
        } else if context_has_weak_code && (message_has_brand || context_has_brand) {
            2
        } else {
            0
        };

        if score == 0 {
            continue;
        }

        let candidate = (score, digit_match.start(), digit_match.as_str().to_string());
        if best_match
            .as_ref()
            .map(|existing| {
                candidate.0 > existing.0 || (candidate.0 == existing.0 && candidate.1 < existing.1)
            })
            .unwrap_or(true)
        {
            best_match = Some(candidate);
        }
    }

    best_match.map(|(_, _, value)| value)
}

fn mailbox_url_candidate_urls(url: &str) -> Vec<String> {
    let mut candidates = vec![url.trim_end_matches('.').to_string()];
    let Ok(parsed) = Url::parse(url) else {
        return candidates;
    };

    for value in parsed
        .query_pairs()
        .map(|(_, value)| value.into_owned())
        .chain(parsed.fragment().map(ToOwned::to_owned))
    {
        if let Some(nested) = URL_REGEX.find(&value) {
            let nested = nested.as_str().trim_end_matches('.').to_string();
            if !candidates.iter().any(|existing| existing == &nested) {
                candidates.push(nested);
            }
        }
    }

    candidates
}

fn mailbox_url_looks_like_direct_invite(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };

    let host = host.to_ascii_lowercase();
    let path = parsed.path().to_ascii_lowercase();
    let query = parsed.query().unwrap_or_default().to_ascii_lowercase();
    let combined = if query.is_empty() {
        format!("{host}{path}")
    } else {
        format!("{host}{path}?{query}")
    };
    let has_invite_action = combined.contains("invite")
        || combined.contains("invitation")
        || combined.contains("accept");
    let has_workspace_context =
        combined.contains("workspace") || host.contains("chatgpt") || host.contains("openai");
    let is_help_like = host.starts_with("help.")
        || host.starts_with("docs.")
        || host.contains("support")
        || path.contains("/articles/")
        || path.contains("/hc/")
        || path.contains("/help/")
        || path.contains("/docs/");

    has_invite_action && has_workspace_context && !is_help_like
}

fn mailbox_url_resolve_invite_target(url: &str) -> Option<String> {
    let candidates = mailbox_url_candidate_urls(url);
    candidates
        .iter()
        .skip(1)
        .find(|candidate| mailbox_url_looks_like_direct_invite(candidate))
        .cloned()
        .or_else(|| {
            candidates
                .into_iter()
                .next()
                .filter(|candidate| mailbox_url_looks_like_direct_invite(candidate))
        })
}

fn mailbox_url_has_brand(url: &str) -> bool {
    mailbox_url_candidate_urls(url)
        .into_iter()
        .any(|candidate| {
            let lower = candidate.to_ascii_lowercase();
            lower.contains("openai") || lower.contains("chatgpt")
        })
}

fn parse_mailbox_code(detail: &MoeMailMessageDetail) -> Option<ParsedMailboxCode> {
    let subject = detail.subject.as_deref().unwrap_or_default();
    let content = detail.content.as_deref().unwrap_or_default();
    let html_text = strip_html_tags(detail.html.as_deref().unwrap_or_default());
    let message_context = normalize_mailbox_text(&format!("{subject}\n{content}\n{html_text}"));
    let message_has_brand = mailbox_text_has_brand(&message_context);

    let subject_text = normalize_mailbox_text(subject);
    let subject_has_brand = mailbox_text_has_brand(&subject_text);
    if subject_has_brand {
        if let Some(value) = extract_mailbox_code_candidate(&subject_text, subject_has_brand) {
            return Some(ParsedMailboxCode {
                value,
                source: "subject".to_string(),
                updated_at: detail
                    .received_at
                    .clone()
                    .unwrap_or_else(|| format_utc_iso(Utc::now())),
            });
        }
    }

    for (source, raw) in [
        ("content", content.to_string()),
        ("html", html_text.clone()),
    ] {
        let normalized = normalize_mailbox_text(&raw);
        if let Some(value) = extract_mailbox_code_candidate(&normalized, message_has_brand) {
            return Some(ParsedMailboxCode {
                value,
                source: source.to_string(),
                updated_at: detail
                    .received_at
                    .clone()
                    .unwrap_or_else(|| format_utc_iso(Utc::now())),
            });
        }
    }

    None
}

fn parse_mailbox_invite(detail: &MoeMailMessageDetail) -> Option<ParsedMailboxInvite> {
    let subject = detail.subject.as_deref().unwrap_or_default().trim();
    if subject.is_empty() {
        return None;
    }

    let stripped_html = strip_html_tags(detail.html.as_deref().unwrap_or_default());
    let subject_text = normalize_mailbox_text(subject);
    let body_text = normalize_mailbox_text(&format!(
        "{}\n{}",
        detail.content.as_deref().unwrap_or_default(),
        stripped_html
    ));
    let subject_has_invite_semantics =
        mailbox_text_contains_any(&subject_text, OAUTH_INVITE_SUBJECT_MARKERS);
    let body_has_invite_semantics =
        mailbox_text_contains_any(&body_text, OAUTH_INVITE_BODY_MARKERS);

    let body_with_urls = format!(
        "{}\n{}",
        detail.content.as_deref().unwrap_or_default(),
        stripped_html
    );
    let copy_value = URL_REGEX
        .find_iter(&body_with_urls)
        .find_map(|value| mailbox_url_resolve_invite_target(value.as_str()))?;
    let body_can_drive_invite = body_has_invite_semantics;
    if !subject_has_invite_semantics && !body_can_drive_invite {
        return None;
    }
    if !mailbox_text_has_brand(&format!("{subject_text}\n{body_text}"))
        && !mailbox_url_has_brand(&copy_value)
    {
        return None;
    }

    Some(ParsedMailboxInvite {
        subject: subject.to_string(),
        copy_label: "invite-link".to_string(),
        copy_value,
        updated_at: detail
            .received_at
            .clone()
            .unwrap_or_else(|| format_utc_iso(Utc::now())),
    })
}

fn parsed_code_from_mailbox_row(row: &OauthMailboxSessionRow) -> Option<ParsedMailboxCode> {
    Some(ParsedMailboxCode {
        value: row.latest_code_value.clone()?,
        source: row.latest_code_source.clone()?,
        updated_at: row.latest_code_updated_at.clone()?,
    })
}

fn parsed_invite_from_mailbox_row(row: &OauthMailboxSessionRow) -> Option<ParsedMailboxInvite> {
    Some(ParsedMailboxInvite {
        subject: row.invite_subject.clone()?,
        copy_value: row.invite_copy_value.clone()?,
        copy_label: row.invite_copy_label.clone()?,
        updated_at: row.invite_updated_at.clone()?,
    })
}

fn mailbox_updated_at_is_newer_or_equal(candidate: &str, baseline: &str) -> bool {
    match (parse_rfc3339_utc(candidate), parse_rfc3339_utc(baseline)) {
        (Some(candidate), Some(baseline)) => candidate >= baseline,
        _ => candidate >= baseline,
    }
}

fn merge_mailbox_code(
    fresh: Option<ParsedMailboxCode>,
    stored: Option<ParsedMailboxCode>,
) -> Option<ParsedMailboxCode> {
    match (fresh, stored) {
        (Some(fresh), Some(stored)) => {
            if mailbox_updated_at_is_newer_or_equal(&fresh.updated_at, &stored.updated_at) {
                Some(fresh)
            } else {
                Some(stored)
            }
        }
        (Some(fresh), None) => Some(fresh),
        (None, Some(stored)) => Some(stored),
        (None, None) => None,
    }
}

fn merge_mailbox_invite(
    fresh: Option<ParsedMailboxInvite>,
    stored: Option<ParsedMailboxInvite>,
) -> Option<ParsedMailboxInvite> {
    match (fresh, stored) {
        (Some(fresh), Some(stored)) => {
            if mailbox_updated_at_is_newer_or_equal(&fresh.updated_at, &stored.updated_at) {
                Some(fresh)
            } else {
                Some(stored)
            }
        }
        (Some(fresh), None) => Some(fresh),
        (None, Some(stored)) => Some(stored),
        (None, None) => None,
    }
}

fn sort_mailbox_messages_desc(messages: &mut [MoeMailMessageSummary]) {
    messages.sort_by(|left, right| right.received_at.cmp(&left.received_at));
}

fn latest_mailbox_message_id(messages: &[MoeMailMessageSummary]) -> Option<String> {
    messages.first().map(|message| message.id.clone())
}

fn collect_unseen_mailbox_messages(
    messages: Vec<MoeMailMessageSummary>,
    last_message_id: Option<&str>,
) -> Vec<MoeMailMessageSummary> {
    let Some(last_message_id) = last_message_id.filter(|value| !value.trim().is_empty()) else {
        return messages;
    };

    let mut unseen = Vec::new();
    for message in messages {
        if message.id == last_message_id {
            break;
        }
        unseen.push(message);
    }
    unseen
}

fn next_mailbox_cursor_after_refresh(
    previous_last_message_id: Option<&str>,
    processed_messages: &[MoeMailMessageSummary],
) -> Option<String> {
    processed_messages
        .first()
        .map(|message| message.id.clone())
        .or_else(|| previous_last_message_id.map(ToOwned::to_owned))
}

async fn resolve_mailbox_message_state(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    messages: &[MoeMailMessageSummary],
) -> Result<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail = moemail_get_message(client, config, remote_email_id, &summary.id).await?;
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok((latest_code, latest_invite))
}

enum MoeMailAttachReadState<T> {
    Readable(T),
    NotReadable,
}

fn moemail_attach_status_is_not_readable(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    )
}

async fn resolve_mailbox_message_state_for_attach(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    messages: &[MoeMailMessageSummary],
) -> Result<MoeMailAttachReadState<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)>> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail =
            match moemail_get_message_for_attach(client, config, remote_email_id, &summary.id)
                .await?
            {
                MoeMailAttachReadState::Readable(detail) => detail,
                MoeMailAttachReadState::NotReadable => {
                    return Ok(MoeMailAttachReadState::NotReadable);
                }
            };
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok(MoeMailAttachReadState::Readable((
        latest_code,
        latest_invite,
    )))
}

async fn moemail_create_email(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
) -> Result<MoeMailGenerateEmailPayload> {
    let local_name = generate_mailbox_local_name().map_err(|(_, message)| anyhow!(message))?;
    moemail_create_email_with_name_and_domain(client, config, &local_name, &config.default_domain)
        .await
}

async fn moemail_create_email_for_address(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    email_address: &str,
) -> Result<MoeMailGenerateEmailPayload> {
    let requested_email = normalize_mailbox_address(email_address)
        .ok_or_else(|| anyhow!("manual moemail address must not be blank"))?;
    let requested_domain = normalize_mailbox_domain(&requested_email)
        .ok_or_else(|| anyhow!("manual moemail domain is invalid"))?;
    let requested_local = mailbox_local_part(&requested_email)
        .ok_or_else(|| anyhow!("manual moemail local part is invalid"))?;
    let generated = moemail_create_email_with_name_and_domain(
        client,
        config,
        requested_local,
        &requested_domain,
    )
    .await?;
    let generated_email = normalize_mailbox_address(&generated.email)
        .ok_or_else(|| anyhow!("generated moemail address must not be blank"))?;
    if generated_email != requested_email {
        bail!(
            "generated moemail address {} does not match requested {}",
            generated.email,
            requested_email
        );
    }
    Ok(generated)
}

async fn moemail_create_email_with_name_and_domain(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    local_name: &str,
    domain: &str,
) -> Result<MoeMailGenerateEmailPayload> {
    let response = client
        .post(
            config
                .base_url
                .join("/api/emails/generate")
                .context("invalid moemail generate endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .json(&json!({
            "name": local_name,
            "expiryTime": DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS * 1000,
            "domain": domain,
        }))
        .send()
        .await
        .context("failed to create moemail mailbox")?
        .error_for_status()
        .context("moemail mailbox creation request failed")?;

    response
        .json::<MoeMailGenerateEmailPayload>()
        .await
        .context("failed to decode moemail create mailbox response")
}

async fn moemail_get_config(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
) -> Result<MoeMailConfigPayload> {
    let response = client
        .get(
            config
                .base_url
                .join("/api/config")
                .context("invalid moemail config endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .context("failed to load moemail config")?
        .error_for_status()
        .context("moemail config request failed")?;

    response
        .json::<MoeMailConfigPayload>()
        .await
        .context("failed to decode moemail config response")
}

async fn moemail_list_emails(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
) -> Result<Vec<MoeMailEmailSummary>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = config
            .base_url
            .join("/api/emails")
            .context("invalid moemail email list endpoint")?;
        if let Some(current_cursor) = cursor.as_deref() {
            url.query_pairs_mut().append_pair("cursor", current_cursor);
        }
        let response = client
            .get(url)
            .header("X-API-Key", config.api_key.as_str())
            .send()
            .await
            .context("failed to list moemail mailboxes")?
            .error_for_status()
            .context("moemail email list request failed")?;
        let payload = response
            .json::<MoeMailEmailListPayload>()
            .await
            .context("failed to decode moemail email list response")?;
        items.extend(payload.emails);
        match payload.next_cursor {
            Some(next_cursor) if !next_cursor.trim().is_empty() => cursor = Some(next_cursor),
            _ => break,
        }
    }
    Ok(items)
}

async fn moemail_list_messages(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
) -> Result<Vec<MoeMailMessageSummary>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}"))
                .context("invalid moemail email detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list moemail messages for {remote_email_id}"))?
        .error_for_status()
        .with_context(|| format!("moemail list messages request failed for {remote_email_id}"))?;

    let payload = response
        .json::<MoeMailMessageListPayload>()
        .await
        .context("failed to decode moemail message list response")?;
    Ok(payload.messages)
}

async fn moemail_list_messages_for_attach(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
) -> Result<MoeMailAttachReadState<Vec<MoeMailMessageSummary>>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}"))
                .context("invalid moemail email detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list moemail messages for {remote_email_id}"))?;
    if moemail_attach_status_is_not_readable(response.status()) {
        return Ok(MoeMailAttachReadState::NotReadable);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("moemail list messages request failed for {remote_email_id}"))?;

    let payload = response
        .json::<MoeMailMessageListPayload>()
        .await
        .context("failed to decode moemail message list response")?;
    Ok(MoeMailAttachReadState::Readable(payload.messages))
}

async fn moemail_get_message(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    message_id: &str,
) -> Result<MoeMailMessageDetail> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}/{message_id}"))
                .context("invalid moemail message detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load moemail message {message_id}"))?
        .error_for_status()
        .with_context(|| format!("moemail message request failed for {message_id}"))?;

    let payload = response
        .json::<MoeMailMessageDetailPayload>()
        .await
        .context("failed to decode moemail message detail response")?;
    Ok(payload.message)
}

async fn moemail_get_message_for_attach(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    message_id: &str,
) -> Result<MoeMailAttachReadState<MoeMailMessageDetail>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}/{message_id}"))
                .context("invalid moemail message detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load moemail message {message_id}"))?;
    if moemail_attach_status_is_not_readable(response.status()) {
        return Ok(MoeMailAttachReadState::NotReadable);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("moemail message request failed for {message_id}"))?;

    let payload = response
        .json::<MoeMailMessageDetailPayload>()
        .await
        .context("failed to decode moemail message detail response")?;
    Ok(MoeMailAttachReadState::Readable(payload.message))
}

async fn moemail_delete_email(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
) -> Result<()> {
    client
        .delete(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}"))
                .context("invalid moemail delete endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to delete moemail mailbox {remote_email_id}"))?
        .error_for_status()
        .with_context(|| format!("moemail delete request failed for {remote_email_id}"))?;
    Ok(())
}

async fn refresh_oauth_mailbox_session_status(
    state: &AppState,
    row: &OauthMailboxSessionRow,
) -> Result<OauthMailboxSessionRow> {
    let config = upstream_mailbox_config(&state.config).map_err(|(_, message)| anyhow!(message))?;
    let mut messages =
        moemail_list_messages(&state.http_clients.shared, config, &row.remote_email_id).await?;
    sort_mailbox_messages_desc(&mut messages);

    let unseen_messages = collect_unseen_mailbox_messages(messages, row.last_message_id.as_deref());
    let (fresh_code, fresh_invite) = resolve_mailbox_message_state(
        &state.http_clients.shared,
        config,
        &row.remote_email_id,
        &unseen_messages,
    )
    .await?;
    let latest_code = merge_mailbox_code(fresh_code, parsed_code_from_mailbox_row(row));
    let latest_invite = merge_mailbox_invite(fresh_invite, parsed_invite_from_mailbox_row(row));
    let next_last_message_id =
        next_mailbox_cursor_after_refresh(row.last_message_id.as_deref(), &unseen_messages);

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_mailbox_sessions
        SET latest_code_value = ?2,
            latest_code_source = ?3,
            latest_code_updated_at = ?4,
            invite_subject = ?5,
            invite_copy_value = ?6,
            invite_copy_label = ?7,
            invite_updated_at = ?8,
            invited = ?9,
            last_message_id = ?10,
            updated_at = ?11
        WHERE session_id = ?1
        "#,
    )
    .bind(&row.session_id)
    .bind(latest_code.as_ref().map(|value| value.value.clone()))
    .bind(latest_code.as_ref().map(|value| value.source.clone()))
    .bind(latest_code.as_ref().map(|value| value.updated_at.clone()))
    .bind(latest_invite.as_ref().map(|value| value.subject.clone()))
    .bind(latest_invite.as_ref().map(|value| value.copy_value.clone()))
    .bind(latest_invite.as_ref().map(|value| value.copy_label.clone()))
    .bind(latest_invite.as_ref().map(|value| value.updated_at.clone()))
    .bind(if latest_invite.is_some() { 1 } else { 0 })
    .bind(next_last_message_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await?;

    load_oauth_mailbox_session(&state.pool, &row.session_id)
        .await?
        .ok_or_else(|| anyhow!("mailbox session disappeared after status refresh"))
}

fn normalize_tag_name(value: &str) -> Result<String, (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "tag name is required".to_string()));
    }
    if trimmed.chars().count() > 48 {
        return Err((
            StatusCode::BAD_REQUEST,
            "tag name must be 48 characters or fewer".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_positive_i64(
    value: Option<i64>,
    field_name: &str,
) -> Result<Option<i64>, (StatusCode, String)> {
    match value {
        Some(number) if number <= 0 => Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be a positive integer"),
        )),
        other => Ok(other),
    }
}

fn normalize_bulk_upstream_account_ids(
    account_ids: &[i64],
) -> Result<Vec<i64>, (StatusCode, String)> {
    let mut normalized = account_ids
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "accountIds must contain at least one positive integer".to_string(),
        ));
    }
    Ok(normalized)
}

fn normalize_upstream_account_list_page(value: Option<usize>) -> usize {
    value.filter(|page| *page > 0).unwrap_or(1)
}

fn normalize_upstream_account_list_page_size(value: Option<usize>) -> usize {
    value
        .filter(|page_size| UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS.contains(page_size))
        .unwrap_or(DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE)
}

#[derive(Debug, Default, Clone, Copy)]
struct LegacyUpstreamAccountStatusFilter {
    work_status: Option<&'static str>,
    enable_status: Option<&'static str>,
    health_status: Option<&'static str>,
    sync_state: Option<&'static str>,
}

fn normalize_upstream_account_work_status_filter(value: Option<&str>) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING => Some(UPSTREAM_ACCOUNT_WORK_STATUS_WORKING),
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED => Some(UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED),
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE => Some(UPSTREAM_ACCOUNT_WORK_STATUS_IDLE),
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED => {
            Some(UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED)
        }
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE => Some(UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE),
        _ => None,
    }
}

fn normalize_upstream_account_enable_status_filter(value: Option<&str>) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED => Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
        UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED => Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
        _ => None,
    }
}

fn normalize_upstream_account_health_status_filter(value: Option<&str>) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL => Some(UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL),
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH => Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE)
        }
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED)
        }
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER | UPSTREAM_ACCOUNT_STATUS_ERROR => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER)
        }
        _ => None,
    }
}

fn collect_normalized_upstream_account_filters(
    values: &[String],
    legacy_value: Option<&'static str>,
    normalize: fn(Option<&str>) -> Option<&'static str>,
) -> Vec<&'static str> {
    let mut normalized = Vec::new();

    for value in values {
        let Some(next_value) = normalize(Some(value.as_str())) else {
            continue;
        };
        if !normalized.contains(&next_value) {
            normalized.push(next_value);
        }
    }

    if normalized.is_empty() {
        if let Some(legacy_value) = legacy_value {
            normalized.push(legacy_value);
        }
    }

    normalized
}

fn normalize_legacy_upstream_account_status_filter(
    value: Option<&str>,
) -> LegacyUpstreamAccountStatusFilter {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    match normalized.as_deref() {
        Some(UPSTREAM_ACCOUNT_STATUS_ACTIVE) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            health_status: Some(UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_STATUS_SYNCING) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            health_status: Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER) | Some(UPSTREAM_ACCOUNT_STATUS_ERROR) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_STATUS_DISABLED) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        _ => LegacyUpstreamAccountStatusFilter::default(),
    }
}

fn normalize_bulk_upstream_account_action(value: &str) -> Result<String, (StatusCode, String)> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DELETE
        | BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP
        | BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS
        | BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS => Ok(normalized),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "unsupported bulk action".to_string(),
        )),
    }
}

fn normalize_tag_rule(
    guard_enabled: bool,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: bool,
    allow_cut_in: bool,
) -> Result<TagRoutingRule, (StatusCode, String)> {
    let lookback_hours = normalize_positive_i64(lookback_hours, "lookbackHours")?;
    let max_conversations = normalize_positive_i64(max_conversations, "maxConversations")?;
    if guard_enabled && (lookback_hours.is_none() || max_conversations.is_none()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "lookbackHours and maxConversations are required when guardEnabled is true".to_string(),
        ));
    }
    Ok(TagRoutingRule {
        guard_enabled,
        lookback_hours: if guard_enabled { lookback_hours } else { None },
        max_conversations: if guard_enabled {
            max_conversations
        } else {
            None
        },
        allow_cut_out,
        allow_cut_in,
    })
}

fn parse_tag_ids_json(raw: Option<&str>) -> Vec<i64> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<i64>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|value| *value > 0)
        .collect()
}

fn encode_tag_ids_json(tag_ids: &[i64]) -> Result<String> {
    serde_json::to_string(tag_ids).context("failed to encode tag ids")
}

fn account_tag_summary_from_row(row: &AccountTagRow) -> AccountTagSummary {
    AccountTagSummary {
        id: row.tag_id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            guard_enabled: row.guard_enabled != 0,
            lookback_hours: row.lookback_hours,
            max_conversations: row.max_conversations,
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
        },
    }
}

fn tag_summary_from_row(row: &TagListRow) -> TagSummary {
    TagSummary {
        id: row.id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            guard_enabled: row.guard_enabled != 0,
            lookback_hours: row.lookback_hours,
            max_conversations: row.max_conversations,
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
        },
        account_count: row.account_count,
        group_count: row.group_count,
        updated_at: row.updated_at.clone(),
    }
}

fn build_effective_routing_rule(tags: &[AccountTagSummary]) -> EffectiveRoutingRule {
    let mut source_tag_ids = Vec::with_capacity(tags.len());
    let mut source_tag_names = Vec::with_capacity(tags.len());
    let mut guard_rules = Vec::new();
    let mut allow_cut_out = true;
    let mut allow_cut_in = true;
    let mut representative_guard: Option<(i64, i64)> = None;

    for tag in tags {
        source_tag_ids.push(tag.id);
        source_tag_names.push(tag.name.clone());
        allow_cut_out &= tag.routing_rule.allow_cut_out;
        allow_cut_in &= tag.routing_rule.allow_cut_in;
        if tag.routing_rule.guard_enabled
            && let (Some(lookback_hours), Some(max_conversations)) = (
                tag.routing_rule.lookback_hours,
                tag.routing_rule.max_conversations,
            )
        {
            guard_rules.push(EffectiveConversationGuard {
                tag_id: tag.id,
                tag_name: tag.name.clone(),
                lookback_hours,
                max_conversations,
            });
            representative_guard = match representative_guard {
                Some((current_hours, current_max))
                    if current_max < max_conversations
                        || (current_max == max_conversations
                            && current_hours >= lookback_hours) =>
                {
                    Some((current_hours, current_max))
                }
                _ => Some((lookback_hours, max_conversations)),
            };
        }
    }

    EffectiveRoutingRule {
        guard_enabled: !guard_rules.is_empty(),
        lookback_hours: representative_guard.map(|(hours, _)| hours),
        max_conversations: representative_guard.map(|(_, max)| max),
        allow_cut_out,
        allow_cut_in,
        source_tag_ids,
        source_tag_names,
        guard_rules,
    }
}

fn effective_account_status(row: &UpstreamAccountRow) -> String {
    if row.enabled == 0 {
        UPSTREAM_ACCOUNT_STATUS_DISABLED.to_string()
    } else {
        row.status.clone()
    }
}

fn derive_upstream_account_enable_status(enabled: bool) -> &'static str {
    if enabled {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED
    } else {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED
    }
}

fn derive_upstream_account_sync_state(enabled: bool, raw_status: &str) -> &'static str {
    if !enabled {
        return UPSTREAM_ACCOUNT_SYNC_STATE_IDLE;
    }
    if raw_status
        .trim()
        .eq_ignore_ascii_case(UPSTREAM_ACCOUNT_STATUS_SYNCING)
    {
        UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING
    } else {
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE
    }
}

fn derive_upstream_account_health_status(
    account_kind: &str,
    enabled: bool,
    raw_status: &str,
    last_error: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> &'static str {
    if !enabled {
        return UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL;
    }
    let status = raw_status.trim().to_ascii_lowercase();
    let error_message = last_error.unwrap_or_default();
    if matches!(
        status.as_str(),
        UPSTREAM_ACCOUNT_STATUS_ACTIVE | UPSTREAM_ACCOUNT_STATUS_SYNCING
    ) && is_transient_route_failure_error(
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    ) {
        return UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL;
    }
    if upstream_account_rate_limit_state_is_current(
        status.as_str(),
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    ) {
        return UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL;
    }
    if status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
        || last_action_reason_code == Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
        || (account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
            && status == UPSTREAM_ACCOUNT_STATUS_ERROR
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message))
    {
        return UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH;
    }
    if is_upstream_unavailable_error_message(error_message) {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE;
    }
    if upstream_account_upstream_rejected_state_is_current(
        status.as_str(),
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    ) {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED;
    }
    if is_upstream_rejected_error_message(error_message) {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED;
    }
    if status == UPSTREAM_ACCOUNT_STATUS_ERROR
        || is_bridge_error_message(error_message)
        || !error_message.trim().is_empty()
    {
        return UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER;
    }
    UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
}

fn is_transient_route_failure_error(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    if last_error_at.is_none() || last_error_at != last_route_failure_at {
        return false;
    }
    let failure_kind = last_route_failure_kind
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if matches!(
        last_action_reason_code,
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
    ) {
        return matches!(failure_kind, Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED));
    }
    route_failure_kind_is_temporary(failure_kind)
        || route_failure_kind_is_rate_limited(failure_kind)
}

fn derive_upstream_account_work_status(
    enabled: bool,
    raw_status: &str,
    health_status: &str,
    sync_state: &str,
    snapshot_exhausted: bool,
    cooldown_until: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
    temporary_route_failure_streak_started_at: Option<&str>,
    last_selected_at: Option<&str>,
    now: DateTime<Utc>,
) -> &'static str {
    if !enabled || sync_state == UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING {
        return UPSTREAM_ACCOUNT_WORK_STATUS_IDLE;
    }
    if snapshot_exhausted
        || upstream_account_quota_exhausted_state_is_current(
            raw_status,
            last_error_at,
            last_route_failure_at,
            last_route_failure_kind,
            last_action_reason_code,
        )
    {
        return UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED;
    }
    if health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL {
        return UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE;
    }
    if upstream_account_degraded_state_is_current(
        raw_status,
        cooldown_until,
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
        temporary_route_failure_streak_started_at,
        now,
    ) {
        return UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED;
    }
    let active_cutoff = now - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES);
    if last_selected_at
        .and_then(parse_rfc3339_utc)
        .is_some_and(|selected_at| selected_at >= active_cutoff)
    {
        return UPSTREAM_ACCOUNT_WORK_STATUS_WORKING;
    }
    UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
}

fn classify_upstream_account_display_status(
    account_kind: &str,
    enabled: bool,
    raw_status: &str,
    last_error: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> &'static str {
    let enable_status = derive_upstream_account_enable_status(enabled);
    if enable_status == UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED {
        return UPSTREAM_ACCOUNT_STATUS_DISABLED;
    }
    let sync_state = derive_upstream_account_sync_state(enabled, raw_status);
    if sync_state == UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING {
        return UPSTREAM_ACCOUNT_STATUS_SYNCING;
    }
    let health_status = derive_upstream_account_health_status(
        account_kind,
        enabled,
        raw_status,
        last_error,
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
        last_action_reason_code,
    );
    if health_status == UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL {
        return UPSTREAM_ACCOUNT_STATUS_ACTIVE;
    }
    health_status
}

fn matches_upstream_account_filters(
    item: &UpstreamAccountSummary,
    work_status_filters: &[&str],
    enable_status_filters: &[&str],
    health_status_filters: &[&str],
    sync_state_filter: Option<&str>,
) -> bool {
    (work_status_filters.is_empty() || work_status_filters.contains(&item.work_status.as_str()))
        && (enable_status_filters.is_empty()
            || enable_status_filters.contains(&item.enable_status.as_str()))
        && (health_status_filters.is_empty()
            || health_status_filters.contains(&item.health_status.as_str()))
        && sync_state_filter.is_none_or(|value| item.sync_state == value)
}

fn build_upstream_account_list_metrics(
    items: &[UpstreamAccountSummary],
) -> UpstreamAccountListMetrics {
    UpstreamAccountListMetrics {
        total: items.len(),
        oauth: items
            .iter()
            .filter(|item| item.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
            .count(),
        api_key: items
            .iter()
            .filter(|item| item.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
            .count(),
        attention: items
            .iter()
            .filter(|item| {
                item.health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
                    || matches!(
                        item.work_status.as_str(),
                        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
                            | UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
                    )
            })
            .count(),
    }
}

fn build_api_key_window(
    limit: Option<f64>,
    unit: Option<&str>,
    window_duration_mins: i64,
) -> Option<RateWindowSnapshot> {
    let limit_text = match limit {
        Some(value) => format!(
            "{} {}",
            format_compact_decimal(value),
            unit.unwrap_or(DEFAULT_API_KEY_LIMIT_UNIT)
        ),
        None => "—".to_string(),
    };
    Some(RateWindowSnapshot {
        used_percent: 0.0,
        used_text: format!("0 {}", unit.unwrap_or(DEFAULT_API_KEY_LIMIT_UNIT)),
        limit_text,
        resets_at: None,
        window_duration_mins,
        actual_usage: None,
    })
}

fn build_window_snapshot(
    used_percent: Option<f64>,
    window_duration_mins: Option<i64>,
    resets_at: Option<&str>,
) -> Option<RateWindowSnapshot> {
    let used_percent = used_percent?;
    let window_duration_mins = window_duration_mins?;
    Some(RateWindowSnapshot {
        used_percent,
        used_text: format!("{}%", format_percent(used_percent)),
        limit_text: format_window_label(window_duration_mins),
        resets_at: resets_at.map(ToOwned::to_owned),
        window_duration_mins,
        actual_usage: None,
    })
}

struct UpstreamAccountActionPayload<'a> {
    action: &'a str,
    source: &'a str,
    reason_code: Option<&'a str>,
    reason_message: Option<&'a str>,
    http_status: Option<StatusCode>,
    failure_kind: Option<&'a str>,
    invoke_id: Option<&'a str>,
    sticky_key: Option<&'a str>,
    occurred_at: &'a str,
}

fn sync_cause_action_source(cause: SyncCause) -> &'static str {
    match cause {
        SyncCause::Manual => UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncCause::Maintenance => UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncCause::PostCreate => UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_POST_CREATE,
    }
}

fn sanitize_account_action_message(message: &str) -> Option<String> {
    let collapsed = message
        .chars()
        .map(|ch| {
            if ch.is_control() && ch != '\n' && ch != '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(240).collect())
}

fn upstream_account_history_retention_days() -> u64 {
    parse_u64_env_var(
        ENV_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
    )
    .unwrap_or(DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS)
}

async fn record_upstream_account_action(
    pool: &Pool<Sqlite>,
    account_id: i64,
    payload: UpstreamAccountActionPayload<'_>,
) -> Result<()> {
    let reason_message = payload
        .reason_message
        .and_then(sanitize_account_action_message);
    let created_at = payload.occurred_at;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, reason_code, reason_message,
            http_status, failure_kind, invoke_id, sticky_key, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(account_id)
    .bind(payload.occurred_at)
    .bind(payload.action)
    .bind(payload.source)
    .bind(payload.reason_code)
    .bind(&reason_message)
    .bind(payload.http_status.map(|value| i64::from(value.as_u16())))
    .bind(payload.failure_kind)
    .bind(payload.invoke_id)
    .bind(payload.sticky_key)
    .bind(created_at)
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_action = ?2,
            last_action_source = ?3,
            last_action_reason_code = ?4,
            last_action_reason_message = ?5,
            last_action_http_status = ?6,
            last_action_invoke_id = ?7,
            last_action_at = ?8
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(payload.action)
    .bind(payload.source)
    .bind(payload.reason_code)
    .bind(&reason_message)
    .bind(payload.http_status.map(|value| i64::from(value.as_u16())))
    .bind(payload.invoke_id)
    .bind(payload.occurred_at)
    .execute(pool)
    .await?;

    let retention_cutoff = format_utc_iso(
        Utc::now() - ChronoDuration::days(upstream_account_history_retention_days() as i64),
    );
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_events
        WHERE account_id = ?1 AND occurred_at < ?2
        "#,
    )
    .bind(account_id)
    .bind(retention_cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

fn message_mentions_http_status(message: &str, status: StatusCode) -> bool {
    let code = status.as_u16();
    let code_text = code.to_string();
    [
        format!("returned {code_text}"),
        format!("responded with {code_text}"),
        format!("{code_text}:"),
        format!("status {code_text}"),
        format!("status code {code_text}"),
        format!(" {code_text} "),
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn extract_status_code_from_error_message(message: &str) -> Option<StatusCode> {
    [
        StatusCode::UNAUTHORIZED,
        StatusCode::PAYMENT_REQUIRED,
        StatusCode::FORBIDDEN,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
    ]
    .into_iter()
    .find(|status| message_mentions_http_status(message, *status))
}

fn classify_sync_failure(
    account_kind: &str,
    error_message: &str,
) -> (
    UpstreamAccountFailureDisposition,
    &'static str,
    Option<&'static str>,
    Option<StatusCode>,
    &'static str,
) {
    if account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
        && is_explicit_reauth_error_message(error_message)
        && !is_scope_permission_error_message(error_message)
        && !is_bridge_error_message(error_message)
    {
        return (
            UpstreamAccountFailureDisposition::HardUnavailable,
            UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
            Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            extract_status_code_from_error_message(error_message),
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
        );
    }

    if let Some(status) = extract_status_code_from_error_message(error_message) {
        let classification =
            classify_pool_account_http_failure(account_kind, status, error_message);
        return (
            classification.disposition,
            classification.reason_code,
            classification.next_account_status,
            Some(status),
            classification.failure_kind,
        );
    }

    let normalized = error_message.to_ascii_lowercase();
    if normalized.contains("failed to request")
        || normalized.contains("timed out")
        || normalized.contains("connection")
        || normalized.contains("transport")
    {
        return (
            UpstreamAccountFailureDisposition::Retryable,
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            None,
            None,
            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        );
    }

    (
        UpstreamAccountFailureDisposition::Retryable,
        UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
        None,
        None,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
    )
}

fn account_reason_is_rate_limited(reason_code: Option<&str>) -> bool {
    account_reason_is_quota_exhausted(reason_code)
}

fn account_reason_is_temporary_failure(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some(
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT
                | UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE
                | UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED
        )
    )
}

fn account_reason_is_upstream_rejected(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some("upstream_http_401" | "upstream_http_402" | "upstream_http_403")
    )
}

fn account_reason_is_quota_exhausted(reason_code: Option<&str>) -> bool {
    matches!(
        reason_code,
        Some(
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED
                | UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED
        )
    )
}

fn status_preserves_current_route_failure(raw_status: &str) -> bool {
    matches!(
        raw_status.trim().to_ascii_lowercase().as_str(),
        UPSTREAM_ACCOUNT_STATUS_ACTIVE | UPSTREAM_ACCOUNT_STATUS_SYNCING
    )
}

fn account_reason_overrides_current_route_failure(
    raw_status: &str,
    reason_code: Option<&str>,
) -> bool {
    matches!(
        reason_code,
        Some(
            UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED
                | UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED
        )
    ) || (matches!(
        reason_code,
        Some(
            UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR
                | UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE
        )
    ) && !status_preserves_current_route_failure(raw_status))
}

fn route_failure_kind_is_rate_limited(failure_kind: Option<&str>) -> bool {
    route_failure_kind_is_quota_exhausted(failure_kind)
}

fn route_failure_kind_is_temporary(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX
                | FORWARD_PROXY_FAILURE_SEND_ERROR
                | FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT
                | FORWARD_PROXY_FAILURE_STREAM_ERROR
                | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
                | PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
        )
    )
}

fn route_failure_kind_is_quota_exhausted(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED
        )
    )
}

fn route_failure_kind_is_upstream_rejected(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH | PROXY_FAILURE_UPSTREAM_HTTP_402)
    )
}

fn route_failure_is_current(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
) -> bool {
    last_error_at.is_some() && last_error_at == last_route_failure_at
}

fn current_route_failure_is_rate_limited(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_rate_limited(last_route_failure_kind)
}

fn current_route_failure_is_temporary(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_temporary(last_route_failure_kind)
}

fn current_route_failure_is_quota_exhausted(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_quota_exhausted(last_route_failure_kind)
}

fn current_route_failure_is_upstream_rejected(
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
) -> bool {
    route_failure_is_current(last_error_at, last_route_failure_at)
        && route_failure_kind_is_upstream_rejected(last_route_failure_kind)
}

fn upstream_account_rate_limit_state_is_current(
    raw_status: &str,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    account_reason_is_rate_limited(last_action_reason_code)
        || (!account_reason_overrides_current_route_failure(raw_status, last_action_reason_code)
            && current_route_failure_is_rate_limited(
                last_error_at,
                last_route_failure_at,
                last_route_failure_kind,
            ))
}

fn account_has_active_cooldown(cooldown_until: Option<&str>, now: DateTime<Utc>) -> bool {
    cooldown_until
        .and_then(parse_rfc3339_utc)
        .is_some_and(|until| until > now)
}

fn upstream_account_degraded_state_is_current(
    raw_status: &str,
    cooldown_until: Option<&str>,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
    temporary_route_failure_streak_started_at: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    if account_reason_overrides_current_route_failure(raw_status, last_action_reason_code) {
        return false;
    }
    let degraded_anchor_at = last_route_failure_at
        .and_then(parse_rfc3339_utc)
        .or_else(|| temporary_route_failure_streak_started_at.and_then(parse_rfc3339_utc));
    if account_has_active_cooldown(cooldown_until, now)
        && route_failure_kind_is_temporary(last_route_failure_kind)
    {
        return true;
    }
    if current_route_failure_is_temporary(
        last_error_at,
        last_route_failure_at,
        last_route_failure_kind,
    ) || account_reason_is_temporary_failure(last_action_reason_code)
        && route_failure_kind_is_temporary(last_route_failure_kind)
    {
        return degraded_anchor_at.is_some_and(|failed_at| {
            failed_at + ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS)
                > now
        });
    }
    false
}

fn upstream_account_quota_exhausted_state_is_current(
    raw_status: &str,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    account_reason_is_quota_exhausted(last_action_reason_code)
        || (!account_reason_overrides_current_route_failure(raw_status, last_action_reason_code)
            && current_route_failure_is_quota_exhausted(
                last_error_at,
                last_route_failure_at,
                last_route_failure_kind,
            ))
}

fn upstream_account_upstream_rejected_state_is_current(
    raw_status: &str,
    last_error_at: Option<&str>,
    last_route_failure_at: Option<&str>,
    last_route_failure_kind: Option<&str>,
    last_action_reason_code: Option<&str>,
) -> bool {
    account_reason_is_upstream_rejected(last_action_reason_code)
        || (!account_reason_overrides_current_route_failure(raw_status, last_action_reason_code)
            && current_route_failure_is_upstream_rejected(
                last_error_at,
                last_route_failure_at,
                last_route_failure_kind,
            ))
}

fn route_failure_kind_requires_manual_api_key_recovery(failure_kind: Option<&str>) -> bool {
    matches!(
        failure_kind
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some(
            PROXY_FAILURE_UPSTREAM_HTTP_AUTH
                | PROXY_FAILURE_UPSTREAM_HTTP_402
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
        )
    )
}

fn should_clear_route_failure_state_after_sync_success(row: &UpstreamAccountRow) -> bool {
    row.status != UPSTREAM_ACCOUNT_STATUS_ACTIVE
        || route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        )
}

fn account_update_requests_manual_recovery(payload: &UpdateUpstreamAccountRequest) -> bool {
    payload.enabled == Some(true)
        || payload
            .api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

async fn set_account_status(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    last_error: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = CASE
                WHEN ?2 = ?6 AND ?3 IS NULL THEN last_error
                ELSE ?3
            END,
            last_error_at = CASE
                WHEN ?2 = ?6 AND ?3 IS NULL THEN last_error_at
                WHEN ?3 IS NULL THEN last_error_at
                ELSE ?4
            END,
            last_route_failure_at = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE last_route_failure_at
            END,
            last_route_failure_kind = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE last_route_failure_kind
            END,
            cooldown_until = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE cooldown_until
            END,
            consecutive_route_failures = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN 0
                ELSE consecutive_route_failures
            END,
            temporary_route_failure_streak_started_at = CASE
                WHEN ?2 = ?5 AND ?3 IS NULL THEN NULL
                ELSE temporary_route_failure_streak_started_at
            END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(last_error)
    .bind(&now_iso)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(UPSTREAM_ACCOUNT_STATUS_SYNCING)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_account_sync_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    route_state: SyncSuccessRouteState,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    match route_state {
        SyncSuccessRouteState::PreserveFailureState => {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_synced_at = ?3,
                    last_successful_sync_at = ?3,
                    last_error = NULL,
                    last_error_at = NULL,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .execute(pool)
            .await?;
        }
        SyncSuccessRouteState::ClearFailureState => {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_synced_at = ?3,
                    last_successful_sync_at = ?3,
                    last_error = NULL,
                    last_error_at = NULL,
                    last_route_failure_at = NULL,
                    last_route_failure_kind = NULL,
                    cooldown_until = NULL,
                    consecutive_route_failures = 0,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?3
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .bind(&now_iso)
            .execute(pool)
            .await?;
        }
    }
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED,
            source,
            reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_OK),
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_account_sync_recovery_blocked(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    status: &str,
    reason_code: &'static str,
    reason_message: &str,
    preserved_error: Option<&str>,
    failure_kind: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = COALESCE(?4, last_error),
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(preserved_error)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_account_sync_hard_unavailable(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    reason_code: &'static str,
    reason_message: &str,
    failure_kind: &'static str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = ?4,
            last_error_at = ?3,
            last_route_failure_at = ?3,
            last_route_failure_kind = ?5,
            cooldown_until = NULL,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
    .bind(&now_iso)
    .bind(reason_message)
    .bind(failure_kind)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(reason_message),
            http_status: None,
            failure_kind: Some(failure_kind),
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_account_sync_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    source: &str,
    status: &str,
    error_message: &str,
    reason_code: &'static str,
    http_status: Option<StatusCode>,
    failure_kind: &'static str,
    preserved_route_failure_kind: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_error = ?4,
            last_error_at = ?3,
            last_route_failure_at = CASE WHEN ?5 IS NULL THEN last_route_failure_at ELSE ?3 END,
            last_route_failure_kind = COALESCE(?5, last_route_failure_kind),
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(&now_iso)
    .bind(error_message)
    .bind(preserved_route_failure_kind)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED,
            source,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status,
            failure_kind: Some(failure_kind),
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn record_classified_account_sync_failure(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    source: &str,
    error_message: &str,
) -> Result<()> {
    let (disposition, reason_code, next_status, http_status, failure_kind) =
        classify_sync_failure(&row.kind, error_message);
    let next_status = match disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR)
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => UPSTREAM_ACCOUNT_STATUS_ACTIVE,
    };
    let preserved_route_failure_kind = row.last_route_failure_kind.as_deref().filter(|_| {
        status_preserves_current_route_failure(&row.status)
            && (upstream_account_quota_exhausted_state_is_current(
                &row.status,
                row.last_error_at.as_deref(),
                row.last_route_failure_at.as_deref(),
                row.last_route_failure_kind.as_deref(),
                row.last_action_reason_code.as_deref(),
            ) || route_failure_kind_is_temporary(row.last_route_failure_kind.as_deref()))
    });
    record_account_sync_failure(
        pool,
        row.id,
        source,
        next_status,
        error_message,
        reason_code,
        http_status,
        failure_kind,
        preserved_route_failure_kind,
    )
    .await
}

async fn record_account_update_action(
    pool: &Pool<Sqlite>,
    account_id: i64,
    message: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_ACCOUNT_UPDATE,
            reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED),
            reason_message: Some(message),
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await
}

async fn exchange_authorization_code(
    client: &Client,
    config: &AppConfig,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse> {
    let url = config
        .upstream_accounts_oauth_issuer
        .join("/oauth/token")
        .context("failed to join OAuth token endpoint")?;
    let response = client
        .post(url)
        .form(&[
            ("grant_type", "authorization_code"),
            (
                "client_id",
                config.upstream_accounts_oauth_client_id.as_str(),
            ),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .context("failed to exchange authorization code")?;
    parse_token_response(response).await
}

async fn client_for_required_proxy_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<Client> {
    let selected_proxy = select_forward_proxy_for_scope(state, scope)
        .await
        .map_err(|err| map_required_group_proxy_selection_error(scope, err))?;
    state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        .context("failed to initialize required forward proxy client")
}

async fn exchange_authorization_code_for_required_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse> {
    let client = client_for_required_proxy_scope(state, scope).await?;
    exchange_authorization_code(&client, &state.config, code, code_verifier, redirect_uri).await
}

async fn refresh_oauth_tokens(
    client: &Client,
    config: &AppConfig,
    refresh_token: &str,
) -> Result<OAuthTokenResponse> {
    let url = config
        .upstream_accounts_oauth_issuer
        .join("/oauth/token")
        .context("failed to join OAuth token endpoint")?;
    let response = client
        .post(url)
        .form(&[
            ("grant_type", "refresh_token"),
            (
                "client_id",
                config.upstream_accounts_oauth_client_id.as_str(),
            ),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("failed to refresh OAuth token")?;
    parse_token_response(response).await
}

async fn refresh_oauth_tokens_for_required_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    refresh_token: &str,
) -> Result<OAuthTokenResponse> {
    let client = client_for_required_proxy_scope(state, scope).await?;
    refresh_oauth_tokens(&client, &state.config, refresh_token).await
}

async fn parse_token_response(response: reqwest::Response) -> Result<OAuthTokenResponse> {
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read OAuth token response body")?;
    if !status.is_success() {
        let detail = extract_error_message(&body);
        bail!("OAuth token endpoint returned {}: {}", status, detail);
    }
    serde_json::from_str(&body).context("failed to decode OAuth token response")
}

fn build_usage_endpoint_url(base_url: &Url) -> Result<Url> {
    let usage_path = if base_url.path().contains("/backend-api") {
        USAGE_PATH_STYLE_CHATGPT
    } else {
        USAGE_PATH_STYLE_CODEX_API
    };
    let base_path = base_url.path().trim_end_matches('/');
    let resolved_path = if base_path.is_empty() || base_path == "/" {
        usage_path.to_string()
    } else {
        format!("{base_path}/{}", usage_path.trim_start_matches('/'))
    };
    let mut url = base_url.clone();
    url.set_path(&resolved_path);
    Ok(url)
}

async fn fetch_usage_snapshot(
    client: &Client,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<NormalizedUsageSnapshot> {
    let primary_result = request_usage_snapshot_with_user_agent(
        client,
        config,
        access_token,
        chatgpt_account_id,
        &config.user_agent,
    )
    .await;

    if primary_result.is_ok() || config.user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
        return primary_result;
    }

    let primary_error = match primary_result {
        Ok(snapshot) => return Ok(snapshot),
        Err(err) => err,
    };

    warn!(
        error = ?primary_error,
        configured_user_agent = %config.user_agent,
        fallback_user_agent = %UPSTREAM_USAGE_BROWSER_USER_AGENT,
        "usage snapshot request failed; retrying with browser user agent"
    );

    request_usage_snapshot_with_user_agent(
        client,
        config,
        access_token,
        chatgpt_account_id,
        UPSTREAM_USAGE_BROWSER_USER_AGENT,
    )
    .await
    .with_context(|| {
        format!(
            "initial usage snapshot attempt with configured user agent failed: {primary_error:#}"
        )
    })
}

fn usage_snapshot_error_is_network_failure(err: &anyhow::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("failed to request usage snapshot")
        || normalized.contains("failed to read usage snapshot response")
        || normalized.contains("timed out")
        || normalized.contains("connection")
        || normalized.contains("transport")
}

async fn fetch_usage_snapshot_via_forward_proxy(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<NormalizedUsageSnapshot> {
    let primary_result = request_usage_snapshot_with_user_agent_via_forward_proxy(
        state,
        scope,
        config,
        access_token,
        chatgpt_account_id,
        &config.user_agent,
    )
    .await;

    if primary_result.is_ok() || config.user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
        return primary_result;
    }

    let primary_error = match primary_result {
        Ok(snapshot) => return Ok(snapshot),
        Err(err) => err,
    };

    warn!(
        error = ?primary_error,
        configured_user_agent = %config.user_agent,
        fallback_user_agent = %UPSTREAM_USAGE_BROWSER_USER_AGENT,
        "usage snapshot request failed; retrying with browser user agent"
    );

    request_usage_snapshot_with_user_agent_via_forward_proxy(
        state,
        scope,
        config,
        access_token,
        chatgpt_account_id,
        UPSTREAM_USAGE_BROWSER_USER_AGENT,
    )
    .await
    .with_context(|| {
        format!(
            "initial usage snapshot attempt with configured user agent failed: {primary_error:#}"
        )
    })
}

async fn request_usage_snapshot_with_user_agent_via_forward_proxy(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    user_agent: &str,
) -> Result<NormalizedUsageSnapshot> {
    let selected_proxy = select_forward_proxy_for_scope(state, scope).await?;
    let client = match state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
    {
        Ok(client) => client,
        Err(err) => {
            record_forward_proxy_scope_result(
                state,
                scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            return Err(err).context("failed to initialize usage snapshot forward proxy client");
        }
    };

    let result = request_usage_snapshot_with_user_agent(
        &client,
        config,
        access_token,
        chatgpt_account_id,
        user_agent,
    )
    .await;

    match &result {
        Ok(_) => {
            record_forward_proxy_scope_result(
                state,
                scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::CompletedRequest,
            )
            .await;
        }
        Err(err) if usage_snapshot_error_is_network_failure(err) => {
            record_forward_proxy_scope_result(
                state,
                scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
        }
        Err(_) => {}
    }

    result
}

async fn request_usage_snapshot_with_user_agent(
    client: &Client,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    user_agent: &str,
) -> Result<NormalizedUsageSnapshot> {
    let url = build_usage_endpoint_url(&config.upstream_accounts_usage_base_url)
        .context("failed to build usage endpoint")?;
    let mut request = client
        .get(url)
        .bearer_auth(access_token)
        .header(header::USER_AGENT, user_agent);
    if let Some(account_id) = chatgpt_account_id
        && !account_id.trim().is_empty()
    {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let response = request
        .send()
        .await
        .context("failed to request usage snapshot")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read usage snapshot response")?;
    if !status.is_success() {
        bail!(
            "usage endpoint returned {}: {}",
            status,
            extract_error_message(&body)
        );
    }
    let value: Value =
        serde_json::from_str(&body).context("failed to decode usage snapshot JSON")?;
    normalize_usage_snapshot(&value)
}

fn normalize_usage_snapshot(value: &Value) -> Result<NormalizedUsageSnapshot> {
    let updated_at = optional_string(value, &["updated_at", "updatedAt"])
        .and_then(|value| parse_rfc3339_utc(&value));
    let limit = value
        .get("rate_limits_by_limit_id")
        .or_else(|| value.get("rateLimitsByLimitId"))
        .and_then(|value| value.get(DEFAULT_USAGE_LIMIT_ID))
        .or_else(|| value.get("rate_limit"))
        .or_else(|| value.get("rateLimit"))
        .unwrap_or(value);
    let primary = normalize_usage_window(
        limit
            .get("primary_window")
            .or_else(|| limit.get("primaryWindow")),
        updated_at,
    );
    let secondary = normalize_usage_window(
        limit
            .get("secondary_window")
            .or_else(|| limit.get("secondaryWindow")),
        updated_at,
    );
    let credits = value
        .get("credits")
        .map(normalize_credits_snapshot)
        .transpose()?;

    Ok(NormalizedUsageSnapshot {
        plan_type: optional_string(value, &["plan_type", "planType"]),
        limit_id: DEFAULT_USAGE_LIMIT_ID.to_string(),
        limit_name: Some(DEFAULT_USAGE_LIMIT_ID.to_string()),
        primary,
        secondary,
        credits,
    })
}

fn normalize_usage_window(
    value: Option<&Value>,
    updated_at: Option<DateTime<Utc>>,
) -> Option<NormalizedUsageWindow> {
    let value = value?;
    let used_percent = value
        .get("used_percent")
        .or_else(|| value.get("usedPercent"))
        .and_then(value_as_f64)?;
    let window_duration_mins = value
        .get("window_duration_mins")
        .or_else(|| value.get("windowDurationMins"))
        .and_then(value_as_i64)
        .or_else(|| {
            value
                .get("limit_window_seconds")
                .or_else(|| value.get("limitWindowSeconds"))
                .and_then(value_as_i64)
                .map(seconds_to_window_minutes)
        })?;
    let resets_at = value
        .get("resets_at")
        .or_else(|| value.get("resetsAt"))
        .and_then(value_as_timestamp)
        .map(format_utc_iso)
        .or_else(|| {
            let base = updated_at.unwrap_or_else(Utc::now);
            value
                .get("reset_after_seconds")
                .or_else(|| value.get("resetAfterSeconds"))
                .and_then(value_as_i64)
                .map(|seconds| format_utc_iso(base + ChronoDuration::seconds(seconds.max(0))))
        });
    Some(NormalizedUsageWindow {
        used_percent,
        window_duration_mins,
        resets_at,
    })
}

fn normalize_credits_snapshot(value: &Value) -> Result<CreditsSnapshot> {
    Ok(CreditsSnapshot {
        has_credits: value
            .get("has_credits")
            .or_else(|| value.get("hasCredits"))
            .and_then(value_as_bool)
            .unwrap_or(false),
        unlimited: value
            .get("unlimited")
            .and_then(value_as_bool)
            .unwrap_or(false),
        balance: value
            .get("balance")
            .or_else(|| value.get("creditBalance"))
            .and_then(value_as_string),
    })
}

fn build_oauth_authorize_url(
    issuer: &Url,
    client_id: &str,
    redirect_uri: &str,
    state_token: &str,
    code_challenge: &str,
) -> Result<String> {
    let mut url = issuer
        .join("/oauth/authorize")
        .context("failed to join OAuth authorize endpoint")?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("audience", DEFAULT_OAUTH_AUDIENCE)
        .append_pair("scope", DEFAULT_OAUTH_SCOPE)
        .append_pair("prompt", DEFAULT_OAUTH_PROMPT)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state_token)
        .append_pair("originator", OAUTH_ORIGINATOR);
    Ok(url.to_string())
}

fn build_manual_callback_redirect_uri() -> Result<String> {
    let mut url =
        Url::parse("http://localhost").context("failed to build localhost callback URL")?;
    let _ = url.set_port(Some(DEFAULT_MANUAL_OAUTH_CALLBACK_PORT));
    url.set_path("/auth/callback");
    Ok(url.to_string())
}

fn derive_secret_key(secret: &str) -> [u8; 32] {
    let digest = Sha256::digest(secret.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

#[allow(deprecated)]
fn encrypt_credentials(key: &[u8; 32], credentials: &StoredCredentials) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let plaintext = serde_json::to_vec(credentials).context("failed to serialize credentials")?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|err| anyhow!("failed to encrypt credentials: {err}"))?;
    serde_json::to_string(&EncryptedCredentialsPayload {
        v: 1,
        nonce: BASE64_STANDARD.encode(nonce),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    })
    .context("failed to encode encrypted credentials payload")
}

#[allow(deprecated)]
fn decrypt_credentials(key: &[u8; 32], payload: &str) -> Result<StoredCredentials> {
    let payload: EncryptedCredentialsPayload =
        serde_json::from_str(payload).context("failed to decode encrypted credentials payload")?;
    if payload.v != 1 {
        bail!(
            "unsupported encrypted credential payload version: {}",
            payload.v
        );
    }
    let nonce = BASE64_STANDARD
        .decode(payload.nonce)
        .context("failed to decode credential nonce")?;
    let ciphertext = BASE64_STANDARD
        .decode(payload.ciphertext)
        .context("failed to decode credential ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let plaintext = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| anyhow!("failed to decrypt credentials: {err}"))?;
    serde_json::from_slice(&plaintext).context("failed to decode credential JSON")
}

fn decode_jwt_payload(token: &str, token_name: &str) -> Result<Vec<u8>> {
    let mut parts = token.split('.');
    let (_header, payload, _sig) = match (parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(sig))
            if !header.is_empty() && !payload.is_empty() && !sig.is_empty() =>
        {
            (header, payload, sig)
        }
        _ => bail!("invalid {token_name} format"),
    };
    URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| BASE64_STANDARD.decode(payload))
        .with_context(|| format!("failed to decode {token_name} payload"))
}

fn parse_chatgpt_jwt_claims(id_token: &str) -> Result<ChatgptJwtClaims> {
    let payload_bytes = decode_jwt_payload(id_token, "id_token")?;
    let claims: ChatgptJwtOuterClaims =
        serde_json::from_slice(&payload_bytes).context("failed to parse id_token payload")?;
    Ok(ChatgptJwtClaims {
        email: claims
            .email
            .or_else(|| claims.profile.and_then(|value| value.email)),
        chatgpt_plan_type: claims
            .auth
            .as_ref()
            .and_then(|value| value.chatgpt_plan_type.clone()),
        chatgpt_user_id: claims.auth.as_ref().and_then(|value| {
            value
                .chatgpt_user_id
                .clone()
                .or_else(|| value.user_id.clone())
        }),
        chatgpt_account_id: claims
            .auth
            .as_ref()
            .and_then(|value| value.chatgpt_account_id.clone()),
    })
}

fn parse_jwt_expiration_utc(token: &str, token_name: &str) -> Option<DateTime<Utc>> {
    let payload_bytes = decode_jwt_payload(token, token_name).ok()?;
    let claims: JwtExpiryClaims = serde_json::from_slice(&payload_bytes).ok()?;
    claims
        .exp
        .and_then(|exp| DateTime::<Utc>::from_timestamp(exp, 0))
}

fn resolve_imported_token_expires_at(
    expired: Option<&str>,
    access_token: &str,
    id_token: &str,
) -> Result<String, String> {
    if let Some(expired) = expired.map(str::trim).filter(|value| !value.is_empty()) {
        return parse_rfc3339_utc(expired)
            .map(format_utc_iso)
            .ok_or_else(|| "expired must be a valid RFC3339 timestamp".to_string());
    }

    parse_jwt_expiration_utc(access_token, "access_token")
        .or_else(|| parse_jwt_expiration_utc(id_token, "id_token"))
        .map(format_utc_iso)
        .ok_or_else(|| "expired is required when token exp is unavailable".to_string())
}

fn render_callback_page(success: bool, title: &str, message: &str) -> String {
    let accent = if success { "#0f8b6f" } else { "#d9485f" };
    let script = if success {
        "setTimeout(() => { try { window.close(); } catch (_) {} }, 1200);"
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <style>
      body {{
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background: radial-gradient(circle at top, rgba(15,139,111,0.12), transparent 45%), #f5f7fb;
        color: #0f172a;
        font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }}
      .card {{
        width: min(92vw, 480px);
        padding: 28px;
        border-radius: 24px;
        background: rgba(255,255,255,0.94);
        box-shadow: 0 24px 80px rgba(15,23,42,0.14);
        border: 1px solid rgba(15,23,42,0.08);
      }}
      .badge {{
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        border-radius: 999px;
        font-size: 13px;
        font-weight: 700;
        color: {accent};
        background: rgba(255,255,255,0.75);
        border: 1px solid rgba(15,23,42,0.08);
      }}
      h1 {{ margin: 16px 0 12px; font-size: 24px; }}
      p {{ margin: 0; line-height: 1.7; color: rgba(15,23,42,0.78); }}
    </style>
  </head>
  <body>
    <main class="card">
      <div class="badge">{badge}</div>
      <h1>{title}</h1>
      <p>{message}</p>
    </main>
    <script>{script}</script>
  </body>
</html>"#,
        title = title,
        accent = accent,
        badge = if success {
            "Codex OAuth connected"
        } else {
            "Codex OAuth failed"
        },
        message = message,
        script = script,
    )
}

fn normalize_required_display_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "displayName is required".to_string(),
        ));
    }
    if value.len() > 120 {
        return Err((
            StatusCode::BAD_REQUEST,
            "displayName must be <= 120 characters".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn validate_group_note_target(
    group_name: Option<&str>,
    has_group_note: bool,
) -> Result<(), (StatusCode, String)> {
    if has_group_note && group_name.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "groupNote requires groupName".to_string(),
        ));
    }
    Ok(())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_upstream_base_url(
    value: Option<String>,
) -> Result<Option<String>, (StatusCode, String)> {
    let Some(raw) = normalize_optional_text(value) else {
        return Ok(None);
    };
    let parsed = Url::parse(&raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "upstreamBaseUrl must be a valid absolute URL".to_string(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.cannot_be_a_base()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "upstreamBaseUrl must be a valid absolute URL".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "upstreamBaseUrl must not include query or fragment".to_string(),
        ));
    }
    Ok(Some(parsed.to_string()))
}

fn resolve_pool_account_upstream_base_url(
    row: &UpstreamAccountRow,
    global_upstream_base_url: &Url,
) -> Result<Url> {
    if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        return oauth_codex_upstream_base_url();
    }
    if row.kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        return Ok(global_upstream_base_url.clone());
    }

    row.upstream_base_url
        .as_deref()
        .map(Url::parse)
        .transpose()
        .context("account upstreamBaseUrl is invalid")?
        .map_or_else(|| Ok(global_upstream_base_url.clone()), Ok)
}

pub(crate) fn canonical_pool_upstream_route_key(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    let scheme_default_port = match normalized.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    if normalized.port().is_some() && normalized.port() == scheme_default_port {
        let _ = normalized.set_port(None);
    }
    let normalized_path = normalized.path().trim_end_matches('/').to_string();
    if normalized_path.is_empty() {
        normalized.set_path("/");
    } else {
        normalized.set_path(&normalized_path);
    }
    normalized.to_string()
}

fn normalize_required_secret(raw: &str, field_name: &str) -> Result<String, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((StatusCode::BAD_REQUEST, format!("{field_name} is required")));
    }
    Ok(value.to_string())
}

fn normalize_limit_unit(value: Option<String>) -> String {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string())
}

fn validate_local_limits(
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
) -> Result<(), (StatusCode, String)> {
    for (label, value) in [
        ("localPrimaryLimit", local_primary_limit),
        ("localSecondaryLimit", local_secondary_limit),
    ] {
        if let Some(value) = value
            && value < 0.0
        {
            return Err((StatusCode::BAD_REQUEST, format!("{label} must be >= 0")));
        }
    }
    Ok(())
}

fn is_import_invalid_error_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    is_explicit_reauth_error_message(message)
        || is_scope_permission_error_message(message)
        || normalized.contains("returned 400")
        || normalized.contains("returned 401")
        || normalized.contains("returned 403")
}

fn persisted_usage_snapshot_is_exhausted(
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    credits_has_credits: Option<bool>,
    credits_unlimited: Option<bool>,
    credits_balance: Option<&str>,
) -> bool {
    let primary_exhausted = primary_used_percent.is_some_and(|value| value >= 100.0);
    let secondary_exhausted = secondary_used_percent.is_some_and(|value| value >= 100.0);
    let credits_exhausted = credits_has_credits.is_some_and(|has_credits| has_credits)
        && !credits_unlimited.unwrap_or(false)
        && credits_balance
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value <= 0.0);
    primary_exhausted || secondary_exhausted || credits_exhausted
}

fn imported_snapshot_is_exhausted(snapshot: &NormalizedUsageSnapshot) -> bool {
    persisted_usage_snapshot_is_exhausted(
        snapshot.primary.as_ref().map(|window| window.used_percent),
        snapshot
            .secondary
            .as_ref()
            .map(|window| window.used_percent),
        snapshot.credits.as_ref().map(|credits| credits.has_credits),
        snapshot.credits.as_ref().map(|credits| credits.unlimited),
        snapshot
            .credits
            .as_ref()
            .and_then(|credits| credits.balance.as_deref()),
    )
}

fn persisted_usage_sample_is_exhausted(sample: Option<&UpstreamAccountSampleRow>) -> bool {
    sample.is_some_and(|sample| {
        persisted_usage_snapshot_is_exhausted(
            sample.primary_used_percent,
            sample.secondary_used_percent,
            sample.credits_has_credits.map(|value| value != 0),
            sample.credits_unlimited.map(|value| value != 0),
            sample.credits_balance.as_deref(),
        )
    })
}

fn routing_candidate_snapshot_is_exhausted(candidate: &AccountRoutingCandidateRow) -> bool {
    persisted_usage_snapshot_is_exhausted(
        candidate.primary_used_percent,
        candidate.secondary_used_percent,
        candidate.credits_has_credits.map(|value| value != 0),
        candidate.credits_unlimited.map(|value| value != 0),
        candidate.credits_balance.as_deref(),
    )
}

fn imported_match_key(email: &str, account_id: &str) -> String {
    let normalized_account_id = account_id.trim().to_ascii_lowercase();
    if !normalized_account_id.is_empty() {
        return format!("account:{normalized_account_id}");
    }
    format!("email:{}", email.trim().to_ascii_lowercase())
}

fn import_match_summary_from_row(row: &UpstreamAccountRow) -> ImportedOauthMatchSummary {
    ImportedOauthMatchSummary {
        account_id: row.id,
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        status: effective_account_status(row),
    }
}

fn normalize_imported_oauth_credentials(
    item: &ImportOauthCredentialFileRequest,
) -> Result<NormalizedImportedOauthCredentials, String> {
    let source_id = normalize_optional_text(Some(item.source_id.clone()))
        .ok_or_else(|| "sourceId is required".to_string())?;
    let file_name = normalize_optional_text(Some(item.file_name.clone()))
        .ok_or_else(|| "fileName is required".to_string())?;
    let content = normalize_optional_text(Some(item.content.clone()))
        .ok_or_else(|| "content is required".to_string())?;
    let parsed: ImportedOauthCredentialsFile =
        serde_json::from_str(&content).map_err(|err| format!("invalid JSON: {err}"))?;
    if !parsed.source_type.eq_ignore_ascii_case("codex") {
        return Err("type must be codex".to_string());
    }
    let email =
        normalize_required_secret(&parsed.email, "email").map_err(|(_, message)| message)?;
    let chatgpt_account_id = normalize_required_secret(&parsed.account_id, "account_id")
        .map_err(|(_, message)| message)?;
    let access_token = normalize_required_secret(&parsed.access_token, "access_token")
        .map_err(|(_, message)| message)?;
    let refresh_token = normalize_required_secret(&parsed.refresh_token, "refresh_token")
        .map_err(|(_, message)| message)?;
    let id_token =
        normalize_required_secret(&parsed.id_token, "id_token").map_err(|(_, message)| message)?;
    let token_expires_at =
        resolve_imported_token_expires_at(parsed.expired.as_deref(), &access_token, &id_token)?;
    let mut claims = parse_chatgpt_jwt_claims(&id_token)
        .map_err(|err| format!("failed to parse id_token: {err}"))?;
    if let Some(jwt_email) = claims.email.as_deref()
        && !jwt_email.trim().eq_ignore_ascii_case(&email)
    {
        return Err("email does not match id_token".to_string());
    }
    if let Some(jwt_account_id) = claims.chatgpt_account_id.as_deref()
        && jwt_account_id.trim() != chatgpt_account_id.trim()
    {
        return Err("account_id does not match id_token".to_string());
    }
    claims.email = Some(email.clone());
    claims.chatgpt_account_id = Some(chatgpt_account_id.clone());
    Ok(NormalizedImportedOauthCredentials {
        source_id,
        file_name,
        email: email.clone(),
        display_name: email,
        chatgpt_account_id,
        token_expires_at,
        credentials: StoredOauthCredentials {
            access_token,
            refresh_token,
            id_token,
            token_type: normalize_optional_text(parsed.token_type)
                .or_else(|| Some("Bearer".to_string())),
        },
        claims,
    })
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn code_challenge_for_verifier(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

fn random_hex(size: usize) -> Result<String, (StatusCode, String)> {
    let mut bytes = vec![0u8; size];
    OsRng.fill_bytes(&mut bytes);
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }
    Ok(output)
}

fn random_base36(size: usize) -> Result<String, (StatusCode, String)> {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    const LETTERS: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
    const DIGITS: &[u8; 10] = b"0123456789";
    let mut rng = OsRng;
    let mut output = Vec::with_capacity(size);
    for _ in 0..size {
        let idx = rng.gen_range(0..ALPHABET.len());
        output.push(ALPHABET[idx]);
    }
    let mut digit_pos = None;
    if size > 0 {
        let pos = rng.gen_range(0..size);
        output[pos] = DIGITS[rng.gen_range(0..DIGITS.len())];
        digit_pos = Some(pos);
    }
    if size > 1 {
        let mut letter_pos = rng.gen_range(0..size);
        if Some(letter_pos) == digit_pos {
            letter_pos = (letter_pos + 1) % size;
        }
        output[letter_pos] = LETTERS[rng.gen_range(0..LETTERS.len())];
    }
    String::from_utf8(output).map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

fn generate_mailbox_local_name() -> Result<String, (StatusCode, String)> {
    const GIVEN_NAMES: &[&str] = &[
        "alex", "emma", "olivia", "liam", "sophia", "noah", "ava", "mia", "ethan", "nora", "lucas",
        "zoe",
    ];
    const FAMILY_NAMES: &[&str] = &[
        "carter", "ng", "morgan", "patel", "reed", "young", "kim", "bennett", "wong", "brooks",
    ];
    const ORG_NAMES: &[&str] = &[
        "northstar",
        "acorn",
        "harbor",
        "summit",
        "evergreen",
        "lattice",
        "brightpath",
        "aurora",
    ];
    const TEAM_NAMES: &[&str] = &[
        "ops", "research", "growth", "support", "finance", "design", "legal", "success",
    ];
    const UNIT_NAMES: &[&str] = &[
        "team", "desk", "hub", "group", "office", "lab", "studio", "center",
    ];

    let mut rng = OsRng;
    let suffix_len = rng.gen_range(3..=5);
    let suffix = random_base36(suffix_len)?;
    let maybe_join = |left: &str, right: &str, rng: &mut OsRng| match rng.gen_range(0..4) {
        0 => format!("{left}{right}"),
        1 => format!("{left}.{right}"),
        _ => format!("{left}-{right}"),
    };
    let mut local = match rng.gen_range(0..3) {
        0 => {
            let base = maybe_join(
                GIVEN_NAMES[rng.gen_range(0..GIVEN_NAMES.len())],
                FAMILY_NAMES[rng.gen_range(0..FAMILY_NAMES.len())],
                &mut rng,
            );
            maybe_join(&base, &suffix, &mut rng)
        }
        1 => {
            let base = maybe_join(
                ORG_NAMES[rng.gen_range(0..ORG_NAMES.len())],
                TEAM_NAMES[rng.gen_range(0..TEAM_NAMES.len())],
                &mut rng,
            );
            maybe_join(&base, &suffix, &mut rng)
        }
        _ => {
            let base = maybe_join(
                TEAM_NAMES[rng.gen_range(0..TEAM_NAMES.len())],
                UNIT_NAMES[rng.gen_range(0..UNIT_NAMES.len())],
                &mut rng,
            );
            maybe_join(&base, &suffix, &mut rng)
        }
    };
    if local.len() < 10 {
        local.push_str(&random_base36(10 - local.len())?);
    }
    Ok(local)
}

fn format_window_label(window_duration_mins: i64) -> String {
    match window_duration_mins {
        300 => "5h quota".to_string(),
        10_080 => "7d quota".to_string(),
        mins if mins % (60 * 24) == 0 => format!("{}d quota", mins / (60 * 24)),
        mins if mins % 60 == 0 => format!("{}h quota", mins / 60),
        mins => format!("{}m quota", mins),
    }
}

fn format_percent(value: f64) -> String {
    if (value.fract()).abs() < 0.05 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

fn format_compact_decimal(value: f64) -> String {
    let rounded = format!("{value:.2}");
    rounded
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn mask_api_key(api_key: &str) -> String {
    if api_key.len() <= 8 {
        return "••••••••".to_string();
    }
    format!("{}••••{}", &api_key[..4], &api_key[api_key.len() - 4..])
}

fn normalize_sticky_key_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(20 | 50 | 100) => raw.unwrap_or(DEFAULT_STICKY_KEY_LIMIT),
        _ => DEFAULT_STICKY_KEY_LIMIT,
    }
}

fn seconds_to_window_minutes(seconds: i64) -> i64 {
    (seconds + 59) / 60
}

fn optional_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(value_as_string)
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(raw) => Some(raw.to_string()),
        _ => None,
    }
}

fn value_as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(raw) => Some(*raw),
        Value::String(raw) => match raw.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(raw) => raw.as_f64(),
        Value::String(raw) => raw.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(raw) => raw.as_i64(),
        Value::String(raw) => raw.parse::<i64>().ok(),
        _ => None,
    }
}

fn value_as_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value_as_i64(value).and_then(|seconds| Utc.timestamp_opt(seconds, 0).single())
}

fn extract_error_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body)
        && let Some(message) = value
            .get("error_description")
            .and_then(value_as_string)
            .or_else(|| value.get("message").and_then(value_as_string))
            .or_else(|| {
                value
                    .get("error")
                    .and_then(|value| value.get("message"))
                    .and_then(value_as_string)
            })
            .or_else(|| value.get("error").and_then(value_as_string))
    {
        return message;
    }
    body.trim().chars().take(240).collect()
}

pub(crate) fn is_scope_permission_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("missing scopes")
        || msg.contains("insufficient permissions for this operation")
        || msg.contains("api.responses.write")
        || msg.contains("api.model.read")
}

fn is_upstream_unavailable_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("failed to contact oauth codex upstream")
        || msg.contains("oauth_upstream_unavailable")
        || msg.contains("failed to contact upstream")
        || msg.contains("connection refused")
        || msg.contains("connection reset")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("temporarily unavailable")
        || msg.contains("service unavailable")
        || msg.contains("bad gateway")
        || msg.contains("gateway timeout")
        || msg.contains("upstream stream error")
        || msg.contains("upstream handshake timed out")
        || msg.contains("upstream response stream reported failure")
        || msg.contains("http 500")
        || msg.contains("http_500")
        || msg.contains("http 502")
        || msg.contains("http_502")
        || msg.contains("http 503")
        || msg.contains("http_503")
        || msg.contains("http 504")
        || msg.contains("http_504")
}

pub(crate) fn is_bridge_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("oauth bridge")
        || msg.contains("token exchange failed")
        || msg.contains("bridge upstream")
        || msg.contains("bridge token")
}

pub(crate) fn is_explicit_reauth_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("invalid_grant")
        || msg.contains("token has been invalidated")
        || msg.contains("token was invalidated")
        || msg.contains("invalidated oauth token")
        || msg.contains("refresh token expired")
        || msg.contains("refresh token revoked")
        || msg.contains("refresh token is invalid")
        || msg.contains("session expired")
        || msg.contains("please sign in again")
        || msg.contains("must sign in again")
        || msg.contains("re-authorize")
        || msg.contains("reauthorize")
}

fn is_upstream_rejected_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    is_scope_permission_error_message(message)
        || msg.contains("oauth_upstream_rejected_request")
        || msg.contains("upstream rejected")
        || msg.contains("forbidden")
        || msg.contains("unauthorized")
        || msg.contains("http 401")
        || msg.contains("http_401")
        || msg.contains("http 403")
        || msg.contains("http_403")
}

fn is_reauth_error(err: &anyhow::Error) -> bool {
    is_explicit_reauth_error_message(&err.to_string())
}

fn internal_error_tuple(err: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn internal_error_html(err: impl ToString) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        render_callback_page(false, "OAuth callback failed", &err.to_string()),
    )
}

pub(crate) fn pool_routing_timeouts_from_config(
    config: &AppConfig,
) -> PoolRoutingTimeoutSettingsResolved {
    PoolRoutingTimeoutSettingsResolved {
        default_first_byte_timeout: config.request_timeout,
        default_send_timeout: config.openai_proxy_handshake_timeout,
        request_read_timeout: config.openai_proxy_request_read_timeout,
        responses_first_byte_timeout: config.pool_upstream_responses_attempt_timeout,
        compact_first_byte_timeout: config.openai_proxy_compact_handshake_timeout,
        responses_stream_timeout: config.pool_upstream_responses_total_timeout,
        compact_stream_timeout: config.pool_upstream_responses_total_timeout,
    }
}

fn normalize_pool_routing_timeout_secs(
    value: Option<u64>,
    field_name: &str,
) -> Result<Option<u64>, (StatusCode, String)> {
    match value {
        None => Ok(None),
        Some(0) => Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be greater than zero"),
        )),
        Some(value) if value > i64::MAX as u64 => Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be less than or equal to {}", i64::MAX),
        )),
        Some(value) => Ok(Some(value)),
    }
}

fn resolve_pool_routing_timeouts_from_row(
    row: &PoolRoutingSettingsRow,
    config: &AppConfig,
) -> PoolRoutingTimeoutSettingsResolved {
    let defaults = pool_routing_timeouts_from_config(config);
    PoolRoutingTimeoutSettingsResolved {
        responses_first_byte_timeout: row
            .responses_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.responses_first_byte_timeout),
        compact_first_byte_timeout: row
            .compact_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.compact_first_byte_timeout),
        responses_stream_timeout: row
            .responses_stream_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.responses_stream_timeout),
        compact_stream_timeout: row
            .compact_stream_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.compact_stream_timeout),
        default_first_byte_timeout: row
            .default_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.default_first_byte_timeout),
        default_send_timeout: row
            .upstream_handshake_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.default_send_timeout),
        request_read_timeout: row
            .request_read_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.request_read_timeout),
    }
}

fn pool_routing_timeouts_response(
    resolved: PoolRoutingTimeoutSettingsResolved,
) -> PoolRoutingTimeoutSettingsResponse {
    PoolRoutingTimeoutSettingsResponse {
        responses_first_byte_timeout_secs: resolved.responses_first_byte_timeout.as_secs(),
        compact_first_byte_timeout_secs: resolved.compact_first_byte_timeout.as_secs(),
        responses_stream_timeout_secs: resolved.responses_stream_timeout.as_secs(),
        compact_stream_timeout_secs: resolved.compact_stream_timeout.as_secs(),
    }
}

async fn load_pool_routing_settings(pool: &Pool<Sqlite>) -> Result<PoolRoutingSettingsRow> {
    sqlx::query_as::<_, PoolRoutingSettingsRow>(
        r#"
        SELECT
            encrypted_api_key,
            masked_api_key,
            primary_sync_interval_secs,
            secondary_sync_interval_secs,
            priority_available_account_cap,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            default_first_byte_timeout_secs,
            upstream_handshake_timeout_secs,
            request_read_timeout_secs
        FROM pool_routing_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

fn resolve_pool_routing_maintenance_settings(
    row: &PoolRoutingSettingsRow,
    config: &AppConfig,
) -> PoolRoutingMaintenanceSettings {
    let primary_sync_interval_secs = row
        .primary_sync_interval_secs
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(config.upstream_accounts_sync_interval.as_secs())
        .max(MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS);
    let secondary_default =
        DEFAULT_UPSTREAM_ACCOUNTS_SECONDARY_SYNC_INTERVAL_SECS.max(primary_sync_interval_secs);
    let secondary_sync_interval_secs = row
        .secondary_sync_interval_secs
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(secondary_default)
        .max(primary_sync_interval_secs)
        .max(MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS);
    let priority_available_account_cap = row
        .priority_available_account_cap
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_UPSTREAM_ACCOUNTS_PRIORITY_AVAILABLE_ACCOUNT_CAP);

    PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs,
        secondary_sync_interval_secs,
        priority_available_account_cap,
    }
}

fn build_pool_routing_settings_response(
    state: &AppState,
    row: &PoolRoutingSettingsRow,
) -> PoolRoutingSettingsResponse {
    let timeouts = resolve_pool_routing_timeouts_from_row(row, &state.config);
    PoolRoutingSettingsResponse {
        writes_enabled: true,
        api_key_configured: row
            .encrypted_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        masked_api_key: row.masked_api_key.clone(),
        maintenance: resolve_pool_routing_maintenance_settings(row, &state.config).into_response(),
        timeouts: pool_routing_timeouts_response(timeouts),
    }
}

fn validate_pool_routing_maintenance_settings(
    settings: PoolRoutingMaintenanceSettings,
) -> Result<(), (StatusCode, String)> {
    if settings.primary_sync_interval_secs < MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "maintenance.primarySyncIntervalSecs must be >= {MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS}"
            ),
        ));
    }
    if settings.secondary_sync_interval_secs < MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "maintenance.secondarySyncIntervalSecs must be >= {MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS}"
            ),
        ));
    }
    if settings.secondary_sync_interval_secs < settings.primary_sync_interval_secs {
        return Err((
            StatusCode::BAD_REQUEST,
            "maintenance.secondarySyncIntervalSecs must be >= maintenance.primarySyncIntervalSecs"
                .to_string(),
        ));
    }
    if settings.priority_available_account_cap == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "maintenance.priorityAvailableAccountCap must be >= 1".to_string(),
        ));
    }
    Ok(())
}

fn merge_pool_routing_maintenance_settings(
    current: PoolRoutingMaintenanceSettings,
    patch: Option<&UpdatePoolRoutingMaintenanceSettingsRequest>,
) -> PoolRoutingMaintenanceSettings {
    let Some(patch) = patch else {
        return current;
    };
    PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: patch
            .primary_sync_interval_secs
            .unwrap_or(current.primary_sync_interval_secs),
        secondary_sync_interval_secs: patch
            .secondary_sync_interval_secs
            .unwrap_or(current.secondary_sync_interval_secs),
        priority_available_account_cap: patch
            .priority_available_account_cap
            .unwrap_or(current.priority_available_account_cap),
    }
}

async fn load_pool_routing_settings_seeded(
    pool: &Pool<Sqlite>,
    _config: &AppConfig,
) -> Result<PoolRoutingSettingsRow> {
    load_pool_routing_settings(pool).await
}

pub(crate) async fn resolve_pool_routing_timeouts(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<PoolRoutingTimeoutSettingsResolved> {
    let row = load_pool_routing_settings_seeded(pool, config).await?;
    Ok(resolve_pool_routing_timeouts_from_row(&row, config))
}

async fn save_pool_routing_settings(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    crypto_key: Option<&[u8; 32]>,
    api_key: Option<&str>,
    timeout_updates: Option<&UpdatePoolRoutingTimeoutSettingsRequest>,
) -> Result<PoolRoutingSettingsRow, (StatusCode, String)> {
    let current = load_pool_routing_settings_seeded(pool, config)
        .await
        .map_err(internal_error_tuple)?;
    let encrypted_api_key = match api_key {
        Some(api_key) => {
            let crypto_key = crypto_key.ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "pool routing secret storage is unavailable".to_string(),
                )
            })?;
            Some(encrypt_secret_value(crypto_key, api_key).map_err(internal_error_tuple)?)
        }
        None => current.encrypted_api_key.clone(),
    };
    let masked_api_key = match api_key {
        Some(api_key) => Some(mask_api_key(api_key)),
        None => current.masked_api_key.clone(),
    };
    let primary_sync_interval_secs = current.primary_sync_interval_secs;
    let secondary_sync_interval_secs = current.secondary_sync_interval_secs;
    let priority_available_account_cap = current.priority_available_account_cap;
    let responses_first_byte_timeout_secs = timeout_updates
        .and_then(|value| value.responses_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.responses_first_byte_timeout_secs);
    let compact_first_byte_timeout_secs = timeout_updates
        .and_then(|value| value.compact_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.compact_first_byte_timeout_secs);
    let responses_stream_timeout_secs = timeout_updates
        .and_then(|value| value.responses_stream_timeout_secs)
        .map(|value| value as i64)
        .or(current.responses_stream_timeout_secs);
    let compact_stream_timeout_secs = timeout_updates
        .and_then(|value| value.compact_stream_timeout_secs)
        .map(|value| value as i64)
        .or(current.compact_stream_timeout_secs);
    let default_first_byte_timeout_secs = current.default_first_byte_timeout_secs;
    let upstream_handshake_timeout_secs = current.upstream_handshake_timeout_secs;
    let request_read_timeout_secs = current.request_read_timeout_secs;
    let now_iso = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET encrypted_api_key = ?2,
            masked_api_key = ?3,
            primary_sync_interval_secs = ?4,
            secondary_sync_interval_secs = ?5,
            priority_available_account_cap = ?6,
            responses_first_byte_timeout_secs = ?7,
            compact_first_byte_timeout_secs = ?8,
            responses_stream_timeout_secs = ?9,
            compact_stream_timeout_secs = ?10,
            default_first_byte_timeout_secs = ?11,
            upstream_handshake_timeout_secs = ?12,
            request_read_timeout_secs = ?13,
            updated_at = ?14
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind(encrypted_api_key)
    .bind(masked_api_key)
    .bind(primary_sync_interval_secs)
    .bind(secondary_sync_interval_secs)
    .bind(priority_available_account_cap)
    .bind(responses_first_byte_timeout_secs)
    .bind(compact_first_byte_timeout_secs)
    .bind(responses_stream_timeout_secs)
    .bind(compact_stream_timeout_secs)
    .bind(default_first_byte_timeout_secs)
    .bind(upstream_handshake_timeout_secs)
    .bind(request_read_timeout_secs)
    .bind(now_iso)
    .execute(pool)
    .await
    .map_err(internal_error_tuple)?;

    load_pool_routing_settings(pool)
        .await
        .map_err(internal_error_tuple)
}

async fn save_pool_routing_api_key(
    pool: &Pool<Sqlite>,
    crypto_key: &[u8; 32],
    api_key: &str,
) -> Result<()> {
    let encrypted_api_key = encrypt_secret_value(crypto_key, api_key)?;
    let masked_api_key = mask_api_key(api_key);
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET encrypted_api_key = ?2,
            masked_api_key = ?3,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind(encrypted_api_key)
    .bind(masked_api_key)
    .bind(now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn save_pool_routing_maintenance_settings(
    pool: &Pool<Sqlite>,
    settings: PoolRoutingMaintenanceSettings,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET primary_sync_interval_secs = ?2,
            secondary_sync_interval_secs = ?3,
            priority_available_account_cap = ?4,
            updated_at = ?5
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind(i64::try_from(settings.primary_sync_interval_secs)?)
    .bind(i64::try_from(settings.secondary_sync_interval_secs)?)
    .bind(i64::try_from(settings.priority_available_account_cap)?)
    .bind(now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn pool_api_key_matches(state: &AppState, api_key: &str) -> Result<bool> {
    let Some(crypto_key) = state.upstream_accounts.crypto_key.as_ref() else {
        return Ok(false);
    };
    let row = load_pool_routing_settings(&state.pool).await?;
    let Some(encrypted_api_key) = row.encrypted_api_key.as_deref() else {
        return Ok(false);
    };
    let decrypted = decrypt_secret_value(crypto_key, encrypted_api_key)?;
    Ok(decrypted == api_key.trim())
}

#[derive(Debug, Clone)]
pub(crate) enum PoolResolvedAuth {
    ApiKey {
        authorization: String,
    },
    Oauth {
        access_token: String,
        chatgpt_account_id: Option<String>,
    },
}

impl PoolResolvedAuth {
    pub(crate) fn authorization_header_value(&self) -> Option<&str> {
        match self {
            Self::ApiKey { authorization } => Some(authorization.as_str()),
            Self::Oauth { .. } => None,
        }
    }

    pub(crate) fn is_oauth(&self) -> bool {
        matches!(self, Self::Oauth { .. })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PoolResolvedAccount {
    pub(crate) account_id: i64,
    pub(crate) display_name: String,
    pub(crate) kind: String,
    pub(crate) auth: PoolResolvedAuth,
    pub(crate) group_name: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) group_upstream_429_retry_enabled: bool,
    pub(crate) group_upstream_429_max_retries: u8,
    pub(crate) upstream_base_url: Url,
    pub(crate) routing_source: PoolRoutingSelectionSource,
}

impl PoolResolvedAccount {
    pub(crate) fn upstream_route_key(&self) -> String {
        canonical_pool_upstream_route_key(&self.upstream_base_url)
    }

    pub(crate) fn effective_group_upstream_429_max_retries(&self) -> u8 {
        normalize_group_upstream_429_retry_metadata(
            self.group_upstream_429_retry_enabled,
            self.group_upstream_429_max_retries,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingSelectionSource {
    StickyReuse,
    FreshAssignment,
}

#[derive(Debug, Clone)]
pub(crate) enum PoolAccountResolution {
    Resolved(PoolResolvedAccount),
    RateLimited,
    DegradedOnly,
    Unavailable,
    NoCandidate,
    BlockedByPolicy(String),
}

#[derive(Debug, Clone)]
enum PoolAccountGroupProxyRoutingReadiness {
    Ready(UpstreamAccountGroupMetadata),
    Blocked(String),
}

async fn load_effective_routing_rule_for_account(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<EffectiveRoutingRule> {
    let tags = load_account_tag_map(pool, &[account_id])
        .await?
        .remove(&account_id)
        .unwrap_or_default();
    Ok(build_effective_routing_rule(&tags))
}

async fn account_accepts_sticky_assignment(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    source_account_id: Option<i64>,
    rule: &EffectiveRoutingRule,
) -> Result<bool> {
    let Some(_) = sticky_key else {
        return Ok(true);
    };
    let is_transfer = source_account_id.is_some_and(|source_id| source_id != account_id);
    let is_new_assignment = source_account_id.is_none();
    if !is_transfer && !is_new_assignment {
        return Ok(true);
    }
    if is_transfer && !rule.allow_cut_in {
        return Ok(false);
    }
    for guard in &rule.guard_rules {
        let current =
            count_recent_account_conversations(pool, account_id, guard.lookback_hours).await?;
        if current >= guard.max_conversations {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn resolve_pool_account_group_proxy_routing_readiness(
    state: &AppState,
    group_name: Option<&str>,
) -> Result<PoolAccountGroupProxyRoutingReadiness> {
    let group_metadata = load_group_metadata(&state.pool, group_name).await?;
    let scope = match required_account_forward_proxy_scope(
        group_name,
        group_metadata.bound_proxy_keys.clone(),
    ) {
        Ok(scope) => scope,
        Err(err) => {
            return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
                err.to_string(),
            ));
        }
    };
    let has_selectable_bound_proxy_keys = match &scope {
        ForwardProxyRouteScope::BoundGroup {
            bound_proxy_keys, ..
        } => {
            let manager = state.forward_proxy.lock().await;
            manager.has_selectable_bound_proxy_keys(bound_proxy_keys)
        }
        ForwardProxyRouteScope::Automatic => true,
    };
    if !has_selectable_bound_proxy_keys {
        let ForwardProxyRouteScope::BoundGroup { group_name, .. } = &scope else {
            unreachable!("strict pool account routing should never fall back to automatic");
        };
        return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
            missing_selectable_group_bound_proxy_error_message(group_name),
        ));
    }
    Ok(PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata))
}

fn summarize_pool_group_proxy_blocked_messages(messages: &[String]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut unique_messages = Vec::new();
    for message in messages {
        let normalized = message.trim();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_string()) {
            unique_messages.push(normalized.to_string());
        }
    }
    let first_message = unique_messages.first()?.clone();
    if unique_messages.len() == 1 {
        return Some(first_message);
    }
    Some(format!(
        "{first_message}; plus {} additional upstream account group routing configuration issue(s)",
        unique_messages.len() - 1
    ))
}

pub(crate) async fn resolve_pool_account_for_request(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
) -> Result<PoolAccountResolution> {
    let now = Utc::now();
    let mut tried = excluded_ids.iter().copied().collect::<HashSet<_>>();
    let mut saw_rate_limited_candidate = false;
    let mut saw_degraded_candidate = false;
    let mut saw_other_non_rate_limited_routing_candidate = false;
    let mut saw_non_routing_candidate = false;
    let mut sticky_route_excluded_by_route_key = false;
    let mut sticky_route_still_reusable = false;
    let mut sticky_route_group_proxy_blocked_message = None;
    let mut group_proxy_blocked_messages = Vec::new();

    let sticky_route = if let Some(sticky_key) = sticky_key {
        load_sticky_route(&state.pool, sticky_key).await?
    } else {
        None
    };
    let sticky_source_id = sticky_route.as_ref().map(|route| route.account_id);
    let sticky_source_rule = if let Some(route) = sticky_route.as_ref() {
        Some(load_effective_routing_rule_for_account(&state.pool, route.account_id).await?)
    } else {
        None
    };

    if let Some(route) = sticky_route.as_ref() {
        if !tried.contains(&route.account_id)
            && let Some(row) = load_upstream_account_row(&state.pool, route.account_id).await?
        {
            tried.insert(route.account_id);
            let sticky_candidate =
                load_account_routing_candidate(&state.pool, route.account_id).await?;
            let sticky_snapshot_exhausted = sticky_candidate
                .as_ref()
                .is_some_and(routing_candidate_snapshot_is_exhausted);
            let sticky_route_is_excluded_by_route_key = resolve_pool_account_upstream_base_url(
                &row,
                &state.config.openai_upstream_base_url,
            )
            .ok()
            .map(|url| canonical_pool_upstream_route_key(&url))
            .is_some_and(|route_key| excluded_upstream_route_keys.contains(&route_key));
            if is_account_selectable_for_sticky_reuse(&row, sticky_snapshot_exhausted, now) {
                sticky_route_still_reusable = true;
                let mut sticky_route_was_excluded = false;
                match resolve_pool_account_group_proxy_routing_readiness(
                    state,
                    row.group_name.as_deref(),
                )
                .await?
                {
                    PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => {
                        if let Some(account) =
                            prepare_pool_account(state, &row, group_metadata).await?
                        {
                            let mut account = account;
                            account.routing_source = PoolRoutingSelectionSource::StickyReuse;
                            if !excluded_upstream_route_keys.contains(&account.upstream_route_key())
                            {
                                return Ok(PoolAccountResolution::Resolved(account));
                            }
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                            if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                            {
                                saw_degraded_candidate = true;
                            } else {
                                saw_other_non_rate_limited_routing_candidate = true;
                            }
                        }
                    }
                    PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                        if sticky_route_is_excluded_by_route_key {
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                        } else {
                            sticky_route_group_proxy_blocked_message = Some(message.clone());
                            group_proxy_blocked_messages.push(message);
                        }
                    }
                }
                if !sticky_route_was_excluded {
                    if sticky_route_group_proxy_blocked_message.is_none() {
                        if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                            saw_degraded_candidate = true;
                        } else {
                            saw_other_non_rate_limited_routing_candidate = true;
                        }
                    }
                }
            } else if is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted) {
                saw_rate_limited_candidate = true;
            } else if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                saw_degraded_candidate = true;
            } else if is_routing_eligible_account(&row) {
                saw_other_non_rate_limited_routing_candidate = true;
            } else if is_pool_account_routing_candidate(&row) {
                // Active accounts without usable credentials are not real
                // routing candidates and should not mask an all-429 pool.
                saw_non_routing_candidate = true;
            }
        }
        if sticky_source_rule
            .as_ref()
            .is_some_and(|rule| !rule.allow_cut_out)
            && sticky_route_still_reusable
            && !sticky_route_excluded_by_route_key
        {
            if let Some(message) = sticky_route_group_proxy_blocked_message {
                return Ok(PoolAccountResolution::BlockedByPolicy(message));
            }
            return Ok(PoolAccountResolution::BlockedByPolicy(
                "sticky conversation cannot cut out of the current account because a tag rule forbids it"
                    .to_string(),
            ));
        }
    }

    let mut candidates = load_account_routing_candidates(&state.pool, &tried).await?;
    for candidate in &mut candidates {
        candidate.in_flight_reservations = pool_routing_reservation_count(state, candidate.id);
    }
    candidates.sort_by(compare_routing_candidates);
    let mut primary_candidates = Vec::new();
    let mut overflow_candidates = Vec::new();
    for candidate in candidates {
        if candidate.effective_load() < candidate.capacity_profile().hard_cap {
            primary_candidates.push(candidate);
        } else {
            overflow_candidates.push(candidate);
        }
    }
    let candidate_passes = if primary_candidates.is_empty() {
        vec![overflow_candidates]
    } else if overflow_candidates.is_empty() {
        vec![primary_candidates]
    } else {
        vec![primary_candidates, overflow_candidates]
    };
    for pass_candidates in candidate_passes {
        for candidate in pass_candidates {
            let Some(row) = load_upstream_account_row(&state.pool, candidate.id).await? else {
                continue;
            };
            let snapshot_exhausted = routing_candidate_snapshot_is_exhausted(&candidate);
            let candidate_route_is_excluded_by_route_key = resolve_pool_account_upstream_base_url(
                &row,
                &state.config.openai_upstream_base_url,
            )
            .ok()
            .map(|url| canonical_pool_upstream_route_key(&url))
            .is_some_and(|route_key| excluded_upstream_route_keys.contains(&route_key));
            if !is_account_selectable_for_fresh_assignment(&row, snapshot_exhausted, now) {
                if is_account_rate_limited_for_routing(&row, snapshot_exhausted) {
                    saw_rate_limited_candidate = true;
                } else if is_account_degraded_for_routing(&row, snapshot_exhausted, now) {
                    saw_degraded_candidate = true;
                } else if is_routing_eligible_account(&row) {
                    saw_other_non_rate_limited_routing_candidate = true;
                } else {
                    saw_non_routing_candidate = true;
                }
                continue;
            }
            let effective_rule =
                load_effective_routing_rule_for_account(&state.pool, row.id).await?;
            if !account_accepts_sticky_assignment(
                &state.pool,
                row.id,
                sticky_key,
                sticky_source_id,
                &effective_rule,
            )
            .await?
            {
                saw_other_non_rate_limited_routing_candidate = true;
                continue;
            }
            let group_metadata = match resolve_pool_account_group_proxy_routing_readiness(
                state,
                row.group_name.as_deref(),
            )
            .await?
            {
                PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => group_metadata,
                PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                    if candidate_route_is_excluded_by_route_key {
                        saw_other_non_rate_limited_routing_candidate = true;
                    } else {
                        group_proxy_blocked_messages.push(message);
                    }
                    continue;
                }
            };
            if let Some(account) = prepare_pool_account(state, &row, group_metadata).await? {
                if excluded_upstream_route_keys.contains(&account.upstream_route_key()) {
                    saw_other_non_rate_limited_routing_candidate = true;
                    continue;
                }
                return Ok(PoolAccountResolution::Resolved(account));
            }
            saw_other_non_rate_limited_routing_candidate = true;
        }
    }

    // Surface concrete group-proxy misconfiguration before generic pool exhaustion
    // when every non-degraded, non-transferable fresh candidate was filtered for
    // that reason, even if the rest of the pool is already rate-limited.
    if !saw_other_non_rate_limited_routing_candidate
        && !saw_degraded_candidate
        && let Some(message) =
            summarize_pool_group_proxy_blocked_messages(&group_proxy_blocked_messages)
    {
        return Ok(PoolAccountResolution::BlockedByPolicy(message));
    }
    if saw_rate_limited_candidate
        && !saw_degraded_candidate
        && !saw_other_non_rate_limited_routing_candidate
    {
        return Ok(PoolAccountResolution::RateLimited);
    }
    if saw_degraded_candidate
        && !saw_rate_limited_candidate
        && !saw_other_non_rate_limited_routing_candidate
        && !saw_non_routing_candidate
    {
        return Ok(PoolAccountResolution::DegradedOnly);
    }
    if saw_other_non_rate_limited_routing_candidate
        || saw_non_routing_candidate
        || (saw_rate_limited_candidate && saw_degraded_candidate)
    {
        return Ok(PoolAccountResolution::Unavailable);
    }

    Ok(PoolAccountResolution::NoCandidate)
}

pub(crate) async fn record_pool_route_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let request_started_at_iso = format_utc_iso(request_started_at_utc);
    let update_result = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_selected_at = COALESCE(last_selected_at, ?3),
            last_error = NULL,
            last_error_at = NULL,
            last_route_failure_at = NULL,
            last_route_failure_kind = NULL,
            cooldown_until = NULL,
            consecutive_route_failures = 0,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
          AND (
                last_route_failure_at IS NULL
                OR last_route_failure_at <= ?4
            )
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .bind(&request_started_at_iso)
    .execute(pool)
    .await?;
    if update_result.rows_affected() == 0 {
        return Ok(());
    }
    if let Some(sticky_key) = sticky_key {
        upsert_sticky_route(pool, sticky_key, account_id, &now_iso).await?;
    }
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: None,
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_pool_route_http_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    if route_http_failure_is_retryable_server_overloaded(status, error_message) {
        return record_pool_route_retryable_overload_failure(
            pool,
            account_id,
            sticky_key,
            error_message,
            invoke_id,
        )
        .await;
    }

    let classification = classify_pool_account_http_failure(account_kind, status, error_message);
    match classification.disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            if let Some(sticky_key) = sticky_key {
                delete_sticky_route(pool, sticky_key).await?;
            }
            let now_iso = format_utc_iso(Utc::now());
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_error = ?3,
                    last_error_at = ?4,
                    last_route_failure_at = ?4,
                    last_route_failure_kind = ?5,
                    cooldown_until = NULL,
                    consecutive_route_failures = consecutive_route_failures + 1,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?4
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(
                classification
                    .next_account_status
                    .unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR),
            )
            .bind(error_message)
            .bind(&now_iso)
            .bind(classification.failure_kind)
            .execute(pool)
            .await?;
            record_upstream_account_action(
                pool,
                account_id,
                UpstreamAccountActionPayload {
                    action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                    source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                    reason_code: Some(classification.reason_code),
                    reason_message: Some(error_message),
                    http_status: Some(status),
                    failure_kind: Some(classification.failure_kind),
                    invoke_id,
                    sticky_key,
                    occurred_at: &now_iso,
                },
            )
            .await?;
            Ok(())
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => {
            let base_secs = if status == StatusCode::TOO_MANY_REQUESTS {
                15
            } else {
                5
            };
            apply_pool_route_cooldown_failure(
                pool,
                account_id,
                sticky_key,
                error_message,
                classification.failure_kind,
                classification.reason_code,
                status,
                base_secs,
                invoke_id,
            )
            .await
        }
    }
}

pub(crate) async fn record_pool_route_retryable_overload_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
        StatusCode::OK,
        5,
        invoke_id,
    )
    .await
}

pub(crate) async fn record_pool_route_transport_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
        StatusCode::BAD_GATEWAY,
        5,
        invoke_id,
    )
    .await
}

pub(crate) async fn build_account_sticky_keys_response(
    pool: &Pool<Sqlite>,
    account_id: i64,
    limit: i64,
) -> Result<AccountStickyKeysResponse> {
    let range_end = Utc::now();
    let range_start = range_end - ChronoDuration::hours(24);
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let routes = load_account_sticky_routes(pool, account_id).await?;
    if routes.is_empty() {
        return Ok(AccountStickyKeysResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            conversations: Vec::new(),
        });
    }

    let attached_keys = routes
        .iter()
        .map(|row| row.sticky_key.clone())
        .collect::<Vec<_>>();
    let aggregates = query_account_sticky_key_aggregates(pool, account_id, &attached_keys).await?;
    let events =
        query_account_sticky_key_events(pool, account_id, &range_start_bound, &attached_keys)
            .await?;

    let mut aggregate_map = aggregates
        .into_iter()
        .map(|row| (row.sticky_key.clone(), row))
        .collect::<HashMap<_, _>>();
    let mut grouped_events: HashMap<String, Vec<AccountStickyKeyRequestPoint>> = HashMap::new();
    for row in events {
        let status = if row.status.trim().is_empty() {
            "unknown".to_string()
        } else {
            row.status.trim().to_string()
        };
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.sticky_key.clone()).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(AccountStickyKeyRequestPoint {
            occurred_at: row.occurred_at,
            status: status.clone(),
            is_success: status.eq_ignore_ascii_case("success"),
            request_tokens,
            cumulative_tokens,
        });
    }

    let mut conversations = routes
        .into_iter()
        .map(|route| {
            let aggregate = aggregate_map.remove(&route.sticky_key);
            let last24h_requests = grouped_events.remove(&route.sticky_key).unwrap_or_default();
            AccountStickyKeyConversation {
                sticky_key: route.sticky_key.clone(),
                request_count: aggregate.as_ref().map(|row| row.request_count).unwrap_or(0),
                total_tokens: aggregate.as_ref().map(|row| row.total_tokens).unwrap_or(0),
                total_cost: aggregate.as_ref().map(|row| row.total_cost).unwrap_or(0.0),
                created_at: aggregate
                    .as_ref()
                    .map(|row| row.created_at.clone())
                    .unwrap_or_else(|| route.created_at.clone()),
                last_activity_at: aggregate
                    .as_ref()
                    .map(|row| row.last_activity_at.clone())
                    .unwrap_or_else(|| route.last_seen_at.clone()),
                last24h_requests,
            }
        })
        .collect::<Vec<_>>();
    conversations.sort_by(|left, right| {
        let left_last_24h = left
            .last24h_requests
            .last()
            .map(|point| point.occurred_at.as_str())
            .unwrap_or("");
        let right_last_24h = right
            .last24h_requests
            .last()
            .map(|point| point.occurred_at.as_str())
            .unwrap_or("");
        right_last_24h
            .cmp(left_last_24h)
            .then_with(|| right.last_activity_at.cmp(&left.last_activity_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.sticky_key.cmp(&right.sticky_key))
    });
    conversations.truncate(limit.max(0) as usize);

    Ok(AccountStickyKeysResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        conversations,
    })
}

async fn load_account_sticky_routes(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Vec<PoolStickyRouteRow>> {
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE account_id = ?1
        ORDER BY updated_at DESC, last_seen_at DESC, sticky_key ASC
        "#,
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn query_account_sticky_key_aggregates(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selected_keys: &[String],
) -> Result<Vec<StickyKeyAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT sticky_key, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MIN(first_seen_at) AS created_at, \
             MAX(last_seen_at) AS last_activity_at \
         FROM upstream_sticky_key_hourly \
         WHERE upstream_account_id = ",
    );
    query.push_bind(account_id).push(" AND sticky_key IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(") GROUP BY sticky_key");

    query
        .build_query_as::<StickyKeyAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn query_account_sticky_key_events(
    pool: &Pool<Sqlite>,
    account_id: i64,
    range_start_bound: &str,
    selected_keys: &[String],
) -> Result<Vec<StickyKeyEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END";
    const ACCOUNT_EXPR: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, 'unknown') AS status, COALESCE(total_tokens, 0) AS request_tokens, ",
    );
    query
        .push(KEY_EXPR)
        .push(" AS sticky_key FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(ACCOUNT_EXPR)
        .push(" = ")
        .push_bind(account_id)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(") ORDER BY sticky_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<StickyKeyEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn prepare_pool_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    group_metadata: UpstreamAccountGroupMetadata,
) -> Result<Option<PoolResolvedAccount>> {
    let Some(crypto_key) = state.upstream_accounts.crypto_key.as_ref() else {
        return Ok(None);
    };
    let Some(encrypted_credentials) = row.encrypted_credentials.as_deref() else {
        return Ok(None);
    };
    let required_proxy_scope = required_account_forward_proxy_scope(
        row.group_name.as_deref(),
        group_metadata.bound_proxy_keys.clone(),
    )?;
    let upstream_base_url =
        resolve_pool_account_upstream_base_url(row, &state.config.openai_upstream_base_url)?;
    let credentials = decrypt_credentials(crypto_key, encrypted_credentials)?;
    match credentials {
        StoredCredentials::ApiKey(value) => Ok(Some(PoolResolvedAccount {
            account_id: row.id,
            display_name: row.display_name.clone(),
            kind: row.kind.clone(),
            auth: PoolResolvedAuth::ApiKey {
                authorization: format!("Bearer {}", value.api_key),
            },
            group_name: row.group_name.clone(),
            bound_proxy_keys: group_metadata.bound_proxy_keys.clone(),
            group_upstream_429_retry_enabled: group_metadata.upstream_429_retry_enabled,
            group_upstream_429_max_retries: group_metadata.upstream_429_max_retries,
            upstream_base_url,
            routing_source: PoolRoutingSelectionSource::FreshAssignment,
        })),
        StoredCredentials::Oauth(mut value) => {
            let expires_at = row.token_expires_at.as_deref().and_then(parse_rfc3339_utc);
            let refresh_due = expires_at
                .map(|expires| {
                    expires
                        <= Utc::now()
                            + ChronoDuration::seconds(
                                state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                            )
                })
                .unwrap_or(true);
            if refresh_due {
                match refresh_oauth_tokens_for_required_scope(
                    state,
                    &required_proxy_scope,
                    &value.refresh_token,
                )
                .await
                {
                    Ok(response) => {
                        value.access_token = response.access_token;
                        if let Some(refresh_token) = response.refresh_token {
                            value.refresh_token = refresh_token;
                        }
                        if let Some(id_token) = response.id_token {
                            value.id_token = id_token;
                        }
                        value.token_type = response.token_type;
                        let token_expires_at = format_utc_iso(
                            Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                        );
                        persist_oauth_credentials(
                            &state.pool,
                            row.id,
                            crypto_key,
                            &value,
                            &token_expires_at,
                        )
                        .await?;
                    }
                    Err(err) if is_reauth_error(&err) => {
                        let err_text = err.to_string();
                        let now_iso = format_utc_iso(Utc::now());
                        sqlx::query(
                            r#"
                            UPDATE pool_upstream_accounts
                            SET status = ?2,
                                last_error = ?3,
                                last_error_at = ?4,
                                last_route_failure_at = ?4,
                                last_route_failure_kind = ?5,
                                cooldown_until = NULL,
                                consecutive_route_failures = consecutive_route_failures + 1,
                                temporary_route_failure_streak_started_at = NULL,
                                updated_at = ?4
                            WHERE id = ?1
                            "#,
                        )
                        .bind(row.id)
                        .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
                        .bind(&err_text)
                        .bind(&now_iso)
                        .bind(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
                        .execute(&state.pool)
                        .await?;
                        record_upstream_account_action(
                            &state.pool,
                            row.id,
                            UpstreamAccountActionPayload {
                                action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                                source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                                reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED),
                                reason_message: Some(&err_text),
                                http_status: None,
                                failure_kind: Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
                                invoke_id: None,
                                sticky_key: None,
                                occurred_at: &now_iso,
                            },
                        )
                        .await?;
                        return Ok(None);
                    }
                    Err(err) => {
                        let err_text = err.to_string();
                        let (disposition, reason_code, next_status, http_status, failure_kind) =
                            classify_sync_failure(&row.kind, &err_text);
                        match disposition {
                            UpstreamAccountFailureDisposition::HardUnavailable => {
                                let now_iso = format_utc_iso(Utc::now());
                                sqlx::query(
                                    r#"
                                    UPDATE pool_upstream_accounts
                                    SET status = ?2,
                                        last_error = ?3,
                                        last_error_at = ?4,
                                        last_route_failure_at = ?4,
                                        last_route_failure_kind = ?5,
                                        cooldown_until = NULL,
                                        consecutive_route_failures = consecutive_route_failures + 1,
                                        temporary_route_failure_streak_started_at = NULL,
                                        updated_at = ?4
                                    WHERE id = ?1
                                    "#,
                                )
                                .bind(row.id)
                                .bind(next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR))
                                .bind(&err_text)
                                .bind(&now_iso)
                                .bind(failure_kind)
                                .execute(&state.pool)
                                .await?;
                                record_upstream_account_action(
                                    &state.pool,
                                    row.id,
                                    UpstreamAccountActionPayload {
                                        action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                                        source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                                        reason_code: Some(reason_code),
                                        reason_message: Some(&err_text),
                                        http_status,
                                        failure_kind: Some(failure_kind),
                                        invoke_id: None,
                                        sticky_key: None,
                                        occurred_at: &now_iso,
                                    },
                                )
                                .await?;
                            }
                            UpstreamAccountFailureDisposition::RateLimited
                            | UpstreamAccountFailureDisposition::Retryable => {
                                apply_pool_route_cooldown_failure(
                                    &state.pool,
                                    row.id,
                                    None,
                                    &err_text,
                                    failure_kind,
                                    reason_code,
                                    http_status.unwrap_or(StatusCode::BAD_GATEWAY),
                                    5,
                                    None,
                                )
                                .await?;
                            }
                        }
                        return Ok(None);
                    }
                }
            }

            Ok(Some(PoolResolvedAccount {
                account_id: row.id,
                display_name: row.display_name.clone(),
                kind: row.kind.clone(),
                auth: PoolResolvedAuth::Oauth {
                    access_token: value.access_token,
                    chatgpt_account_id: row.chatgpt_account_id.clone(),
                },
                group_name: row.group_name.clone(),
                bound_proxy_keys: group_metadata.bound_proxy_keys,
                group_upstream_429_retry_enabled: group_metadata.upstream_429_retry_enabled,
                group_upstream_429_max_retries: group_metadata.upstream_429_max_retries,
                upstream_base_url,
                routing_source: PoolRoutingSelectionSource::FreshAssignment,
            }))
        }
    }
}

fn is_account_selectable_for_sticky_reuse(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    if !is_routing_eligible_account(row) || snapshot_exhausted {
        return false;
    }
    !account_has_active_cooldown(row.cooldown_until.as_deref(), now)
}

fn is_account_selectable_for_fresh_assignment(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    is_account_selectable_for_sticky_reuse(row, snapshot_exhausted, now)
        && !is_account_degraded_for_routing(row, snapshot_exhausted, now)
}

fn is_account_degraded_for_routing(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    is_routing_eligible_account(row)
        && !snapshot_exhausted
        && upstream_account_degraded_state_is_current(
            &row.status,
            row.cooldown_until.as_deref(),
            row.last_error_at.as_deref(),
            row.last_route_failure_at.as_deref(),
            row.last_route_failure_kind.as_deref(),
            row.last_action_reason_code.as_deref(),
            row.temporary_route_failure_streak_started_at.as_deref(),
            now,
        )
}

fn is_pool_account_routing_candidate(row: &UpstreamAccountRow) -> bool {
    row.provider == UPSTREAM_ACCOUNT_PROVIDER_CODEX
        && row.enabled != 0
        && row.status == UPSTREAM_ACCOUNT_STATUS_ACTIVE
}

fn is_routing_eligible_account(row: &UpstreamAccountRow) -> bool {
    is_pool_account_routing_candidate(row) && row.encrypted_credentials.is_some()
}

fn is_account_rate_limited_for_routing(row: &UpstreamAccountRow, snapshot_exhausted: bool) -> bool {
    if row.provider != UPSTREAM_ACCOUNT_PROVIDER_CODEX
        || row.enabled == 0
        || row.encrypted_credentials.is_none()
    {
        return false;
    }
    let quota_exhausted_hard_stop =
        route_failure_kind_is_quota_exhausted(row.last_route_failure_kind.as_deref());
    snapshot_exhausted
        || quota_exhausted_hard_stop
        || account_reason_is_rate_limited(row.last_action_reason_code.as_deref())
}

async fn load_account_routing_candidates(
    pool: &Pool<Sqlite>,
    excluded_ids: &HashSet<i64>,
) -> Result<Vec<AccountRoutingCandidateRow>> {
    let active_sticky_cutoff = format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES),
    );
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.plan_type
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS plan_type,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_window_minutes,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_window_minutes,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            account.local_primary_limit,
            account.local_secondary_limit,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance,
            account.last_selected_at,
            (
                SELECT COUNT(*)
                FROM pool_sticky_routes route
                WHERE route.account_id = account.id
                  AND route.last_seen_at >=
        "#,
    );
    query.push_bind(&active_sticky_cutoff).push(
        r#"
            ) AS active_sticky_conversations
        FROM pool_upstream_accounts account
        WHERE account.provider = 
        "#,
    );
    query
        .push_bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .push(" AND account.enabled = 1");
    if !excluded_ids.is_empty() {
        query.push(" AND account.id NOT IN (");
        {
            let mut separated = query.separated(", ");
            for account_id in excluded_ids {
                separated.push_bind(account_id);
            }
        }
        query.push(")");
    }
    query.push(" ORDER BY account.id ASC");

    query
        .build_query_as::<AccountRoutingCandidateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn load_account_routing_candidate(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<AccountRoutingCandidateRow>> {
    sqlx::query_as::<_, AccountRoutingCandidateRow>(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.plan_type
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS plan_type,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_window_minutes,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_window_minutes,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            account.local_primary_limit,
            account.local_secondary_limit,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance,
            account.last_selected_at,
            (
                SELECT COUNT(*)
                FROM pool_sticky_routes route
                WHERE route.account_id = account.id
                  AND route.last_seen_at >= ?2
            ) AS active_sticky_conversations
        FROM pool_upstream_accounts account
        WHERE account.id = ?1
        "#,
    )
    .bind(account_id)
    .bind(format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES),
    ))
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn compare_routing_candidates(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
) -> std::cmp::Ordering {
    compare_routing_candidates_at(lhs, rhs, Utc::now())
}

fn compare_routing_candidates_at(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
    now: DateTime<Utc>,
) -> std::cmp::Ordering {
    let lhs_capacity = lhs.capacity_profile();
    let rhs_capacity = rhs.capacity_profile();
    let lhs_over_soft_limit = lhs.effective_load() > lhs_capacity.soft_limit;
    let rhs_over_soft_limit = rhs.effective_load() > rhs_capacity.soft_limit;
    let lhs_score = lhs.scarcity_score(now);
    let rhs_score = rhs.scarcity_score(now);
    lhs_over_soft_limit
        .cmp(&rhs_over_soft_limit)
        .then_with(|| {
            lhs_score
                .partial_cmp(&rhs_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.effective_load().cmp(&rhs.effective_load()))
        .then_with(|| lhs.last_selected_at.cmp(&rhs.last_selected_at))
        .then_with(|| lhs.id.cmp(&rhs.id))
}

pub(crate) async fn record_account_selected(pool: &Pool<Sqlite>, account_id: i64) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_selected_at = ?2,
            updated_at = ?2
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn record_compact_support_observation(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    if !matches!(
        status,
        COMPACT_SUPPORT_STATUS_SUPPORTED | COMPACT_SUPPORT_STATUS_UNSUPPORTED
    ) {
        return Ok(());
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET compact_support_status = ?2,
            compact_support_observed_at = ?3,
            compact_support_reason = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(now_iso)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

async fn apply_pool_route_cooldown_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    base_secs: i64,
    invoke_id: Option<&str>,
) -> Result<()> {
    let row = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    let now = Utc::now();
    let continuing_temporary_streak = row.consecutive_route_failures > 0
        && route_failure_kind_is_temporary(row.last_route_failure_kind.as_deref());
    let next_failures = if continuing_temporary_streak {
        row.consecutive_route_failures.max(0) + 1
    } else {
        1
    };
    let streak_started_at = if continuing_temporary_streak {
        row.temporary_route_failure_streak_started_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .or_else(|| {
                row.last_route_failure_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc)
            })
            .unwrap_or(now)
    } else {
        now
    };
    let should_start_cooldown = next_failures >= POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD
        || now.signed_duration_since(streak_started_at).num_seconds()
            >= POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS;
    if should_start_cooldown && let Some(sticky_key) = sticky_key {
        delete_sticky_route(pool, sticky_key).await?;
    }
    let exponent = (next_failures - 1).clamp(0, 5) as u32;
    let cooldown_secs = (base_secs * (1_i64 << exponent)).min(300);
    let now_iso = format_utc_iso(now);
    let streak_started_at_iso = format_utc_iso(streak_started_at);
    let cooldown_until =
        should_start_cooldown.then(|| format_utc_iso(now + ChronoDuration::seconds(cooldown_secs)));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = ?4,
            last_route_failure_at = ?4,
            last_route_failure_kind = ?5,
            cooldown_until = ?6,
            consecutive_route_failures = ?7,
            temporary_route_failure_streak_started_at = ?8,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(error_message)
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(cooldown_until)
    .bind(next_failures)
    .bind(streak_started_at_iso)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: if should_start_cooldown {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED
            } else {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE
            },
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status: Some(http_status),
            failure_kind: Some(failure_kind),
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn load_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
) -> Result<Option<PoolStickyRouteRow>> {
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn upsert_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (
            sticky_key, account_id, created_at, updated_at, last_seen_at
        ) VALUES (?1, ?2, ?3, ?3, ?3)
        ON CONFLICT(sticky_key) DO UPDATE SET
            account_id = excluded.account_id,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn delete_sticky_route(pool: &Pool<Sqlite>, sticky_key: &str) -> Result<()> {
    sqlx::query("DELETE FROM pool_sticky_routes WHERE sticky_key = ?1")
        .bind(sticky_key)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(deprecated)]
fn encrypt_secret_value(key: &[u8; 32], value: &str) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|err| anyhow!("failed to encrypt secret: {err}"))?;
    serde_json::to_string(&EncryptedCredentialsPayload {
        v: 1,
        nonce: BASE64_STANDARD.encode(nonce),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    })
    .context("failed to encode encrypted secret payload")
}

#[allow(deprecated)]
fn decrypt_secret_value(key: &[u8; 32], payload: &str) -> Result<String> {
    let payload: EncryptedCredentialsPayload =
        serde_json::from_str(payload).context("failed to decode encrypted secret payload")?;
    if payload.v != 1 {
        bail!(
            "unsupported encrypted secret payload version: {}",
            payload.v
        );
    }
    let nonce = BASE64_STANDARD
        .decode(payload.nonce)
        .context("failed to decode secret nonce")?;
    let ciphertext = BASE64_STANDARD
        .decode(payload.ciphertext)
        .context("failed to decode secret ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let plaintext = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| anyhow!("failed to decrypt secret: {err}"))?;
    String::from_utf8(plaintext).context("failed to decode decrypted secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    };
    use sqlx::SqlitePool;
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, atomic::AtomicUsize},
        time::Duration,
    };
    use tokio::{
        net::TcpListener,
        sync::{Mutex, Notify},
        time::timeout,
    };

    fn test_summary_with_statuses(
        work_status: &str,
        enable_status: &str,
        health_status: &str,
        sync_state: &str,
    ) -> UpstreamAccountSummary {
        UpstreamAccountSummary {
            id: 1,
            kind: UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX.to_string(),
            provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
            display_name: "Test account".to_string(),
            group_name: Some("alpha".to_string()),
            is_mother: false,
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
            display_status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
            enabled: enable_status == UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
            work_status: work_status.to_string(),
            enable_status: enable_status.to_string(),
            health_status: health_status.to_string(),
            sync_state: sync_state.to_string(),
            email: Some("tester@example.com".to_string()),
            chatgpt_account_id: Some("acct_test".to_string()),
            plan_type: Some("pro".to_string()),
            masked_api_key: None,
            last_synced_at: None,
            last_successful_sync_at: None,
            last_activity_at: None,
            active_conversation_count: 0,
            last_error: None,
            last_error_at: None,
            last_action: None,
            last_action_source: None,
            last_action_reason_code: None,
            last_action_reason_message: None,
            last_action_http_status: None,
            last_action_invoke_id: None,
            last_action_at: None,
            token_expires_at: None,
            primary_window: None,
            secondary_window: None,
            credits: None,
            local_limits: None,
            compact_support: CompactSupportState {
                status: "unknown".to_string(),
                observed_at: None,
                reason: None,
            },
            duplicate_info: None,
            tags: vec![],
            effective_routing_rule: EffectiveRoutingRule {
                guard_enabled: false,
                lookback_hours: None,
                max_conversations: None,
                allow_cut_out: true,
                allow_cut_in: true,
                source_tag_ids: vec![],
                source_tag_names: vec![],
                guard_rules: vec![],
            },
        }
    }

    #[test]
    fn derive_secret_key_is_stable() {
        let lhs = derive_secret_key("alpha");
        let rhs = derive_secret_key("alpha");
        assert_eq!(lhs, rhs);
    }

    #[test]
    fn credential_round_trip_works() {
        let key = derive_secret_key("top-secret");
        let encrypted = encrypt_credentials(
            &key,
            &StoredCredentials::ApiKey(StoredApiKeyCredentials {
                api_key: "sk-test-1234".to_string(),
            }),
        )
        .expect("encrypt credentials");
        let decrypted = decrypt_credentials(&key, &encrypted).expect("decrypt credentials");
        let StoredCredentials::ApiKey(value) = decrypted else {
            panic!("expected API key credentials")
        };
        assert_eq!(value.api_key, "sk-test-1234");
    }

    #[test]
    fn deserialize_optional_field_distinguishes_missing_null_and_value() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload {
            #[serde(default, deserialize_with = "deserialize_optional_field")]
            upstream_base_url: OptionalField<String>,
        }

        let missing: Payload = serde_json::from_value(json!({})).expect("deserialize missing");
        assert_eq!(missing.upstream_base_url, OptionalField::Missing);

        let null_value: Payload =
            serde_json::from_value(json!({ "upstreamBaseUrl": null })).expect("deserialize null");
        assert_eq!(null_value.upstream_base_url, OptionalField::Null);

        let string_value: Payload = serde_json::from_value(json!({
            "upstreamBaseUrl": "https://proxy.example.com/gateway"
        }))
        .expect("deserialize string");
        assert_eq!(
            string_value.upstream_base_url,
            OptionalField::Value("https://proxy.example.com/gateway".to_string())
        );
    }

    #[test]
    fn list_query_deserializes_repeated_status_filters() {
        let query = parse_list_upstream_accounts_query(
            &"/api/pool/upstream-accounts?workStatus=working&workStatus=rate_limited&workStatus=unavailable&enableStatus=enabled&healthStatus=normal&healthStatus=needs_reauth"
                .parse()
                .expect("parse uri"),
        )
        .expect("deserialize repeated filters");

        assert_eq!(
            query.work_status,
            vec![
                UPSTREAM_ACCOUNT_WORK_STATUS_WORKING.to_string(),
                UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED.to_string(),
                UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE.to_string(),
            ]
        );
        assert_eq!(
            query.enable_status,
            vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED.to_string()]
        );
        assert_eq!(
            query.health_status,
            vec![
                UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string(),
                UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH.to_string(),
            ]
        );
    }

    #[test]
    fn list_query_keeps_single_status_filter_compatible() {
        let query = parse_list_upstream_accounts_query(
            &"/api/pool/upstream-accounts?workStatus=idle&enableStatus=disabled&healthStatus=normal"
                .parse()
                .expect("parse uri"),
        )
        .expect("deserialize single filters");

        assert_eq!(
            query.work_status,
            vec![UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string()]
        );
        assert_eq!(
            query.enable_status,
            vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED.to_string()]
        );
        assert_eq!(
            query.health_status,
            vec![UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string()]
        );
    }

    #[test]
    fn explicit_split_filters_override_legacy_status_mapping() {
        let enable_filters = collect_normalized_upstream_account_filters(
            &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED.to_string()],
            Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
            normalize_upstream_account_enable_status_filter,
        );
        assert_eq!(enable_filters, vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED]);

        let health_filters = collect_normalized_upstream_account_filters(
            &[UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string()],
            Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            normalize_upstream_account_health_status_filter,
        );
        assert_eq!(health_filters, vec![UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL]);
    }

    #[test]
    fn matches_upstream_account_filters_uses_or_within_each_dimension() {
        let item = test_summary_with_statuses(
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
        );

        assert!(matches_upstream_account_filters(
            &item,
            &[
                UPSTREAM_ACCOUNT_WORK_STATUS_WORKING,
                UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            ],
            &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED],
            &[
                UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
                UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
            ],
            Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
        ));

        assert!(!matches_upstream_account_filters(
            &item,
            &[UPSTREAM_ACCOUNT_WORK_STATUS_WORKING],
            &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED],
            &[UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL],
            Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
        ));
    }

    #[test]
    fn normalize_imported_oauth_credentials_accepts_codex_export_json() {
        let item = ImportOauthCredentialFileRequest {
            source_id: "file-1".to_string(),
            file_name: "2q5q6m3ow4a@duckmail.sbs.json".to_string(),
            content: json!({
                "type": "codex",
                "email": "2q5q6m3ow4a@duckmail.sbs",
                "account_id": "acct_imported",
                "expired": "2026-03-20T00:00:00Z",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "id_token": test_id_token(
                    "2q5q6m3ow4a@duckmail.sbs",
                    Some("acct_imported"),
                    Some("user_imported"),
                    Some("team"),
                ),
                "last_refresh": "2026-03-18T00:00:00Z"
            })
            .to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
        assert_eq!(normalized.source_id, "file-1");
        assert_eq!(normalized.file_name, "2q5q6m3ow4a@duckmail.sbs.json");
        assert_eq!(normalized.email, "2q5q6m3ow4a@duckmail.sbs");
        assert_eq!(normalized.chatgpt_account_id, "acct_imported");
        assert_eq!(normalized.display_name, "2q5q6m3ow4a@duckmail.sbs");
        assert_eq!(
            normalized.claims.chatgpt_user_id.as_deref(),
            Some("user_imported")
        );
    }

    #[test]
    fn normalize_imported_oauth_credentials_uses_access_token_exp_when_expired_blank() {
        let access_exp = 1_777_777_777;
        let id_exp = 1_666_666_666;
        let item = ImportOauthCredentialFileRequest {
            source_id: "file-blank-expired".to_string(),
            file_name: "blank-expired.json".to_string(),
            content: json!({
                "type": "codex",
                "email": "blank-expired@duckmail.sbs",
                "account_id": "acct_blank_expired",
                "expired": "",
                "access_token": test_jwt_token(json!({ "exp": access_exp })),
                "refresh_token": "refresh-token",
                "id_token": test_jwt_token(json!({
                    "exp": id_exp,
                    "email": "blank-expired@duckmail.sbs",
                    "https://api.openai.com/auth": {
                        "chatgpt_account_id": "acct_blank_expired",
                        "chatgpt_user_id": "user_blank_expired",
                        "chatgpt_plan_type": "team"
                    }
                }))
            })
            .to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
        assert_eq!(normalized.token_expires_at, "2026-05-03T03:09:37Z");
    }

    #[test]
    fn normalize_imported_oauth_credentials_uses_id_token_exp_when_expired_missing() {
        let id_exp = 1_666_666_666;
        let item = ImportOauthCredentialFileRequest {
            source_id: "file-missing-expired".to_string(),
            file_name: "missing-expired.json".to_string(),
            content: json!({
                "type": "codex",
                "email": "missing-expired@duckmail.sbs",
                "account_id": "acct_missing_expired",
                "access_token": "opaque-access-token",
                "refresh_token": "refresh-token",
                "id_token": test_jwt_token(json!({
                    "exp": id_exp,
                    "email": "missing-expired@duckmail.sbs",
                    "https://api.openai.com/auth": {
                        "chatgpt_account_id": "acct_missing_expired",
                        "chatgpt_user_id": "user_missing_expired",
                        "chatgpt_plan_type": "team"
                    }
                }))
            })
            .to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
        assert_eq!(normalized.token_expires_at, "2022-10-25T02:57:46Z");
    }

    #[test]
    fn normalize_imported_oauth_credentials_rejects_non_empty_invalid_expired() {
        let item = ImportOauthCredentialFileRequest {
            source_id: "file-invalid-expired".to_string(),
            file_name: "invalid-expired.json".to_string(),
            content: json!({
                "type": "codex",
                "email": "invalid-expired@duckmail.sbs",
                "account_id": "acct_invalid_expired",
                "expired": "not-a-date",
                "access_token": test_jwt_token(json!({ "exp": 1_777_777_777 })),
                "refresh_token": "refresh-token",
                "id_token": test_jwt_token(json!({
                    "exp": 1_666_666_666,
                    "email": "invalid-expired@duckmail.sbs",
                    "https://api.openai.com/auth": {
                        "chatgpt_account_id": "acct_invalid_expired",
                        "chatgpt_user_id": "user_invalid_expired",
                        "chatgpt_plan_type": "team"
                    }
                }))
            })
            .to_string(),
        };

        let error = normalize_imported_oauth_credentials(&item)
            .expect_err("expected invalid expired timestamp");
        assert_eq!(error, "expired must be a valid RFC3339 timestamp");
    }

    #[test]
    fn normalize_imported_oauth_credentials_rejects_missing_expired_without_token_exp() {
        let item = ImportOauthCredentialFileRequest {
            source_id: "file-missing-expired-no-exp".to_string(),
            file_name: "missing-expired-no-exp.json".to_string(),
            content: json!({
                "type": "codex",
                "email": "missing-expired-no-exp@duckmail.sbs",
                "account_id": "acct_missing_expired_no_exp",
                "access_token": "opaque-access-token",
                "refresh_token": "refresh-token",
                "id_token": test_id_token(
                    "missing-expired-no-exp@duckmail.sbs",
                    Some("acct_missing_expired_no_exp"),
                    Some("user_missing_expired_no_exp"),
                    Some("team"),
                )
            })
            .to_string(),
        };

        let error = normalize_imported_oauth_credentials(&item)
            .expect_err("expected missing expiry to be rejected");
        assert_eq!(error, "expired is required when token exp is unavailable");
    }

    #[test]
    fn normalize_imported_oauth_credentials_rejects_id_token_mismatch() {
        let item = ImportOauthCredentialFileRequest {
            source_id: "file-2".to_string(),
            file_name: "mismatch.json".to_string(),
            content: json!({
                "type": "codex",
                "email": "mismatch@duckmail.sbs",
                "account_id": "acct_imported",
                "expired": "2026-03-20T00:00:00Z",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "id_token": test_id_token(
                    "different@duckmail.sbs",
                    Some("acct_imported"),
                    Some("user_imported"),
                    Some("team"),
                )
            })
            .to_string(),
        };

        let error = normalize_imported_oauth_credentials(&item)
            .expect_err("expected imported oauth mismatch");
        assert_eq!(error, "email does not match id_token");
    }

    #[tokio::test]
    async fn imported_oauth_validation_job_caches_successful_probe_for_import_reuse() {
        let binding = ResolvedRequiredGroupProxyBinding {
            group_name: "import-group".to_string(),
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
        };
        let job = Arc::new(ImportedOauthValidationJob::new(
            ImportedOauthValidationResponse {
                input_files: 1,
                unique_in_input: 1,
                duplicate_in_input: 0,
                rows: vec![ImportedOauthValidationRow {
                    source_id: "source-1".to_string(),
                    file_name: "alpha.json".to_string(),
                    email: None,
                    chatgpt_account_id: None,
                    display_name: None,
                    token_expires_at: None,
                    matched_account: None,
                    status: "pending".to_string(),
                    detail: None,
                    attempts: 0,
                }],
            },
            &binding,
        ));
        let normalized = NormalizedImportedOauthCredentials {
            source_id: "source-1".to_string(),
            file_name: "alpha.json".to_string(),
            email: "alpha@duckmail.sbs".to_string(),
            display_name: "alpha@duckmail.sbs".to_string(),
            chatgpt_account_id: "acct_alpha".to_string(),
            token_expires_at: "2026-03-20T00:00:00Z".to_string(),
            credentials: StoredOauthCredentials {
                access_token: "access-token".to_string(),
                refresh_token: "refresh-token".to_string(),
                id_token: test_id_token(
                    "alpha@duckmail.sbs",
                    Some("acct_alpha"),
                    Some("user_alpha"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            },
            claims: test_claims("alpha@duckmail.sbs", Some("acct_alpha"), Some("user_alpha")),
        };
        let probe = ImportedOauthProbeOutcome {
            token_expires_at: "2026-03-20T00:00:00Z".to_string(),
            credentials: normalized.credentials.clone(),
            claims: normalized.claims.clone(),
            usage_snapshot: None,
            exhausted: false,
            usage_snapshot_warning: Some(
                "usage snapshot unavailable during validation".to_string(),
            ),
        };

        update_imported_oauth_validation_job_row(
            &job,
            0,
            ImportedOauthValidationRow {
                source_id: "source-1".to_string(),
                file_name: "alpha.json".to_string(),
                email: Some("alpha@duckmail.sbs".to_string()),
                chatgpt_account_id: Some("acct_alpha".to_string()),
                display_name: Some("alpha@duckmail.sbs".to_string()),
                token_expires_at: Some("2026-03-20T00:00:00Z".to_string()),
                matched_account: None,
                status: IMPORT_VALIDATION_STATUS_OK.to_string(),
                detail: probe.usage_snapshot_warning.clone(),
                attempts: 1,
            },
            Some(ImportedOauthValidatedImportData { normalized, probe }),
        )
        .await;

        let cached = job
            .validated_imports
            .lock()
            .await
            .get("source-1")
            .cloned()
            .expect("cached validated import");
        assert_eq!(cached.normalized.email, "alpha@duckmail.sbs");
        assert_eq!(cached.normalized.chatgpt_account_id, "acct_alpha");
        assert_eq!(cached.probe.credentials.refresh_token, "refresh-token");
    }

    #[tokio::test]
    async fn create_bulk_upstream_account_sync_job_reuses_existing_running_job() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let snapshot = BulkUpstreamAccountSyncSnapshot {
            job_id: "running-job".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
            rows: vec![BulkUpstreamAccountSyncRow {
                account_id: 5,
                display_name: "Existing OAuth".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING.to_string(),
                detail: None,
            }],
        };
        let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
        state
            .upstream_accounts
            .insert_bulk_sync_job(
                snapshot.job_id.clone(),
                Arc::new(BulkUpstreamAccountSyncJob::new(snapshot.clone())),
            )
            .await;

        let response = create_bulk_upstream_account_sync_job(
            State(state.clone()),
            HeaderMap::new(),
            Json(BulkUpstreamAccountSyncJobRequest {
                account_ids: vec![9, 11],
            }),
        )
        .await
        .expect("reuse running bulk sync job")
        .0;

        assert_eq!(response.job_id, "running-job");
        assert_eq!(response.snapshot.job_id, "running-job");
        assert_eq!(response.snapshot.rows.len(), 1);
        assert_eq!(response.snapshot.rows[0].account_id, 5);
        assert_eq!(response.counts.total, counts.total);
        assert_eq!(response.counts.completed, counts.completed);
        assert_eq!(state.upstream_accounts.bulk_sync_jobs.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn finish_bulk_sync_job_completed_exposes_completed_status_in_events_and_response() {
        let job = Arc::new(BulkUpstreamAccountSyncJob::new(
            BulkUpstreamAccountSyncSnapshot {
                job_id: "job-completed".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
                rows: vec![BulkUpstreamAccountSyncRow {
                    account_id: 5,
                    display_name: "Existing OAuth".to_string(),
                    status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED.to_string(),
                    detail: None,
                }],
            },
        ));
        let mut receiver = job.broadcaster.subscribe();

        finish_bulk_upstream_account_sync_job_completed(&job).await;

        match receiver.recv().await.expect("completed event") {
            BulkUpstreamAccountSyncJobEvent::Completed(payload) => {
                assert_eq!(
                    payload.snapshot.status,
                    BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
                );
                assert_eq!(payload.counts.completed, 1);
                assert_eq!(payload.counts.failed, 0);
                assert_eq!(payload.counts.skipped, 0);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response =
            build_bulk_upstream_account_sync_job_response("job-completed".to_string(), &job).await;
        assert_eq!(
            response.snapshot.status,
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
        );
    }

    #[tokio::test]
    async fn finish_bulk_sync_job_failed_exposes_failed_status_in_events_and_response() {
        let job = Arc::new(BulkUpstreamAccountSyncJob::new(
            BulkUpstreamAccountSyncSnapshot {
                job_id: "job-failed".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
                rows: vec![BulkUpstreamAccountSyncRow {
                    account_id: 5,
                    display_name: "Existing OAuth".to_string(),
                    status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED.to_string(),
                    detail: Some("upstream rejected".to_string()),
                }],
            },
        ));
        let mut receiver = job.broadcaster.subscribe();

        finish_bulk_upstream_account_sync_job_failed(&job, "job failed".to_string()).await;

        match receiver.recv().await.expect("failed event") {
            BulkUpstreamAccountSyncJobEvent::Failed(payload) => {
                assert_eq!(
                    payload.snapshot.status,
                    BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
                );
                assert_eq!(payload.counts.failed, 1);
                assert_eq!(payload.error, "job failed");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response =
            build_bulk_upstream_account_sync_job_response("job-failed".to_string(), &job).await;
        assert_eq!(
            response.snapshot.status,
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
        );
    }

    #[tokio::test]
    async fn finish_bulk_sync_job_cancelled_exposes_cancelled_status_in_events_and_response() {
        let job = Arc::new(BulkUpstreamAccountSyncJob::new(
            BulkUpstreamAccountSyncSnapshot {
                job_id: "job-cancelled".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
                rows: vec![BulkUpstreamAccountSyncRow {
                    account_id: 5,
                    display_name: "Existing OAuth".to_string(),
                    status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED.to_string(),
                    detail: Some("disabled accounts cannot be synced".to_string()),
                }],
            },
        ));
        let mut receiver = job.broadcaster.subscribe();

        finish_bulk_upstream_account_sync_job_cancelled(&job).await;

        match receiver.recv().await.expect("cancelled event") {
            BulkUpstreamAccountSyncJobEvent::Cancelled(payload) => {
                assert_eq!(
                    payload.snapshot.status,
                    BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
                );
                assert_eq!(payload.counts.skipped, 1);
                assert_eq!(payload.counts.completed, 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response =
            build_bulk_upstream_account_sync_job_response("job-cancelled".to_string(), &job).await;
        assert_eq!(
            response.snapshot.status,
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
        );
    }

    #[test]
    fn imported_snapshot_is_exhausted_when_any_limit_is_full_or_credits_are_empty() {
        let primary_exhausted = NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "limit-primary".to_string(),
            limit_name: Some("Primary".to_string()),
            primary: Some(NormalizedUsageWindow {
                used_percent: 100.0,
                window_duration_mins: 300,
                resets_at: Some("2026-03-20T05:00:00Z".to_string()),
            }),
            secondary: None,
            credits: None,
        };
        assert!(imported_snapshot_is_exhausted(&primary_exhausted));

        let credits_exhausted = NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "limit-credits".to_string(),
            limit_name: Some("Credits".to_string()),
            primary: Some(NormalizedUsageWindow {
                used_percent: 42.0,
                window_duration_mins: 300,
                resets_at: Some("2026-03-20T05:00:00Z".to_string()),
            }),
            secondary: Some(NormalizedUsageWindow {
                used_percent: 12.0,
                window_duration_mins: 10_080,
                resets_at: Some("2026-03-27T00:00:00Z".to_string()),
            }),
            credits: Some(CreditsSnapshot {
                has_credits: true,
                unlimited: false,
                balance: Some("0".to_string()),
            }),
        };
        assert!(imported_snapshot_is_exhausted(&credits_exhausted));
    }

    #[tokio::test]
    async fn resolve_pool_account_upstream_base_url_only_overrides_api_key_accounts() {
        let _upstream_lock = crate::oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        crate::oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;

        fn build_row(kind: &str, upstream_base_url: Option<&str>) -> UpstreamAccountRow {
            UpstreamAccountRow {
                id: 1,
                kind: kind.to_string(),
                provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
                display_name: "Test".to_string(),
                group_name: None,
                is_mother: 0,
                note: None,
                status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                enabled: 1,
                email: None,
                chatgpt_account_id: None,
                chatgpt_user_id: None,
                plan_type: None,
                plan_type_observed_at: None,
                masked_api_key: None,
                encrypted_credentials: None,
                token_expires_at: None,
                last_refreshed_at: None,
                last_synced_at: None,
                last_successful_sync_at: None,
                last_activity_at: None,
                last_error: None,
                last_error_at: None,
                last_action: None,
                last_action_source: None,
                last_action_reason_code: None,
                last_action_reason_message: None,
                last_action_http_status: None,
                last_action_invoke_id: None,
                last_action_at: None,
                last_selected_at: None,
                last_route_failure_at: None,
                last_route_failure_kind: None,
                cooldown_until: None,
                consecutive_route_failures: 0,
                temporary_route_failure_streak_started_at: None,
                compact_support_status: None,
                compact_support_observed_at: None,
                compact_support_reason: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                upstream_base_url: upstream_base_url.map(str::to_string),
                created_at: "2026-03-15T00:00:00Z".to_string(),
                updated_at: "2026-03-15T00:00:00Z".to_string(),
            }
        }

        let global = Url::parse("https://api.openai.com/").expect("global upstream base url");
        let override_url = "https://proxy.example.com/gateway";
        crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse("https://chatgpt.com/backend-api/codex").expect("oauth codex base"),
        )
        .await;

        let oauth_row = build_row(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX, Some(override_url));
        let oauth_resolved = resolve_pool_account_upstream_base_url(&oauth_row, &global)
            .expect("resolve oauth upstream base url");
        assert_eq!(
            oauth_resolved.as_str(),
            "https://chatgpt.com/backend-api/codex"
        );

        let api_key_row = build_row(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX, Some(override_url));
        let api_key_resolved = resolve_pool_account_upstream_base_url(&api_key_row, &global)
            .expect("resolve api key upstream base url");
        assert_eq!(
            api_key_resolved.as_str(),
            "https://proxy.example.com/gateway"
        );
    }

    #[test]
    fn parse_chatgpt_jwt_claims_extracts_identity_fields() {
        let payload = json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "pro",
                "chatgpt_user_id": "user_123",
                "chatgpt_account_id": "org_123"
            }
        });
        let encoded = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("{encoded}.{body}.{encoded}");
        let claims = parse_chatgpt_jwt_claims(&token).expect("parse token");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("pro"));
        assert_eq!(claims.chatgpt_user_id.as_deref(), Some("user_123"));
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("org_123"));
    }

    #[test]
    fn build_usage_endpoint_url_preserves_backend_api_prefix() {
        let base = Url::parse("https://chatgpt.com/backend-api").expect("chatgpt base");
        let resolved = build_usage_endpoint_url(&base).expect("resolved usage url");
        assert_eq!(
            resolved.as_str(),
            "https://chatgpt.com/backend-api/wham/usage"
        );

        let base_with_slash =
            Url::parse("https://chatgpt.com/backend-api/").expect("chatgpt base with slash");
        let resolved_with_slash =
            build_usage_endpoint_url(&base_with_slash).expect("resolved usage url");
        assert_eq!(
            resolved_with_slash.as_str(),
            "https://chatgpt.com/backend-api/wham/usage"
        );
    }

    #[test]
    fn normalize_usage_snapshot_reads_windows_and_resets() {
        let payload = json!({
            "planType": "pro",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                },
                "secondaryWindow": {
                    "usedPercent": 18.5,
                    "windowDurationMins": 10080,
                    "resetsAt": 1771927200
                }
            },
            "credits": {
                "hasCredits": true,
                "unlimited": false,
                "balance": "9.99"
            }
        });
        let snapshot = normalize_usage_snapshot(&payload).expect("normalize snapshot");
        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        assert_eq!(
            snapshot.primary.as_ref().map(|value| value.used_percent),
            Some(42.0)
        );
        assert_eq!(
            snapshot.secondary.as_ref().map(|value| value.used_percent),
            Some(18.5)
        );
        assert_eq!(
            snapshot
                .credits
                .as_ref()
                .and_then(|value| value.balance.clone())
                .as_deref(),
            Some("9.99")
        );
    }

    fn usage_snapshot_test_config(base_url: &str, user_agent: &str) -> AppConfig {
        AppConfig {
            openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
            database_path: PathBuf::from(":memory:"),
            poll_interval: Duration::from_secs(10),
            request_timeout: Duration::from_secs(5),
            pool_upstream_responses_attempt_timeout: Duration::from_secs(
                DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
            ),
            pool_upstream_responses_total_timeout: Duration::from_secs(
                DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
            ),
            openai_proxy_handshake_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
            ),
            openai_proxy_compact_handshake_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS,
            ),
            openai_proxy_request_read_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
            ),
            openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
            proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
            proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
            proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
            proxy_raw_dir: PathBuf::from("target/proxy-raw-tests"),
            proxy_raw_compression: DEFAULT_PROXY_RAW_COMPRESSION,
            proxy_raw_hot_secs: DEFAULT_PROXY_RAW_HOT_SECS,
            xray_binary: DEFAULT_XRAY_BINARY.to_string(),
            xray_runtime_dir: PathBuf::from("target/xray-forward-tests"),
            forward_proxy_algo: ForwardProxyAlgo::V1,
            max_parallel_polls: 2,
            shared_connection_parallelism: 1,
            http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
            cors_allowed_origins: Vec::new(),
            list_limit_max: 100,
            user_agent: user_agent.to_string(),
            static_dir: None,
            retention_enabled: DEFAULT_RETENTION_ENABLED,
            retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
            retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
            retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
            retention_catchup_budget: Duration::from_secs(DEFAULT_RETENTION_CATCHUP_BUDGET_SECS),
            archive_dir: PathBuf::from("target/archive-tests"),
            codex_invocation_archive_layout: DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT,
            codex_invocation_archive_segment_granularity:
                DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
            invocation_archive_codec: DEFAULT_INVOCATION_ARCHIVE_CODEC,
            invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
            invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
            invocation_archive_ttl_days: DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS,
            forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
            pool_upstream_request_attempts_retention_days:
                DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
            pool_upstream_request_attempts_archive_ttl_days:
                DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
            stats_source_snapshots_retention_days: DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
            quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
            crs_stats: None,
            upstream_accounts_oauth_client_id: DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID
                .to_string(),
            upstream_accounts_oauth_issuer: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER)
                .expect("valid oauth issuer"),
            upstream_accounts_usage_base_url: Url::parse(base_url).expect("valid usage base url"),
            upstream_accounts_login_session_ttl: Duration::from_secs(
                DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
            ),
            upstream_accounts_sync_interval: Duration::from_secs(
                DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
            ),
            upstream_accounts_refresh_lead_time: Duration::from_secs(
                DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
            ),
            upstream_accounts_history_retention_days:
                DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
            upstream_accounts_moemail: None,
        }
    }

    #[tokio::test]
    async fn fetch_usage_snapshot_retries_with_browser_user_agent() {
        #[derive(Clone)]
        struct UsageSnapshotTestState {
            requests: Arc<Mutex<Vec<String>>>,
        }

        async fn handler(
            State(state): State<UsageSnapshotTestState>,
            headers: HeaderMap,
        ) -> (StatusCode, String) {
            let user_agent = headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            state.requests.lock().await.push(user_agent.clone());
            if user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
                (
                    StatusCode::OK,
                    json!({
                        "planType": "pro",
                        "rateLimit": {
                            "primaryWindow": {
                                "usedPercent": 12,
                                "windowDurationMins": 300,
                                "resetsAt": 1771322400
                            }
                        }
                    })
                    .to_string(),
                )
            } else {
                (
                    StatusCode::FORBIDDEN,
                    json!({ "detail": "blocked user agent" }).to_string(),
                )
            }
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(UsageSnapshotTestState {
                requests: requests.clone(),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });

        let client = Client::builder().build().expect("client");
        let config = usage_snapshot_test_config(
            &format!("http://{addr}/backend-api"),
            "codex-vibe-monitor/0.2.0",
        );

        let snapshot = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
            .await
            .expect("fetch usage snapshot");

        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        let recorded = requests.lock().await.clone();
        assert_eq!(
            recorded,
            vec![
                "codex-vibe-monitor/0.2.0".to_string(),
                UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
            ]
        );

        server.abort();
    }

    #[test]
    fn build_manual_callback_redirect_uri_targets_localhost() {
        let redirect = build_manual_callback_redirect_uri().expect("redirect uri");
        assert!(redirect.starts_with("http://localhost:"));
        assert!(redirect.ends_with("/auth/callback"));
    }

    #[test]
    fn parse_manual_oauth_callback_accepts_expected_redirect() {
        let query = parse_manual_oauth_callback(
            "http://localhost:37891/auth/callback?code=test-code&state=test-state",
            "http://localhost:37891/auth/callback",
        )
        .expect("callback query");
        assert_eq!(query.code.as_deref(), Some("test-code"));
        assert_eq!(query.state.as_deref(), Some("test-state"));
    }

    #[test]
    fn build_oauth_authorize_url_requests_official_scopes_and_audience() {
        let url = build_oauth_authorize_url(
            &Url::parse("https://auth.openai.com").expect("issuer"),
            "client-id",
            "http://localhost:1455/auth/callback",
            "state-token",
            "challenge",
        )
        .expect("build authorize url");
        let parsed = Url::parse(&url).expect("parse authorize url");
        let query = parsed.query_pairs().into_owned().collect::<HashMap<_, _>>();
        let scope = query
            .get("scope")
            .cloned()
            .expect("scope should be present");
        let scope_parts = scope.split_whitespace().collect::<Vec<_>>();

        assert_eq!(
            query.get("audience").map(String::as_str),
            Some(DEFAULT_OAUTH_AUDIENCE)
        );
        assert_eq!(
            query.get("prompt").map(String::as_str),
            Some(DEFAULT_OAUTH_PROMPT)
        );
        assert!(scope_parts.contains(&"openid"));
        assert!(scope_parts.contains(&"profile"));
        assert!(scope_parts.contains(&"email"));
        assert!(scope_parts.contains(&"offline_access"));
        assert_eq!(scope_parts.len(), 4);
    }

    #[test]
    fn is_reauth_error_requires_explicit_invalidated_signal() {
        assert!(is_reauth_error(&anyhow!(
            "OAuth token endpoint returned 400: invalid_grant"
        )));
        assert!(is_reauth_error(&anyhow!(
            "Authentication token has been invalidated, please sign in again"
        )));
        assert!(!is_reauth_error(&anyhow!(
            "usage endpoint returned 401: Missing scopes: api.responses.write"
        )));
        assert!(!is_reauth_error(&anyhow!(
            "pool upstream responded with 403: You have insufficient permissions for this operation."
        )));
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("ensure schema");
        pool
    }

    fn test_required_group_bound_proxy_keys() -> Vec<String> {
        vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
    }

    fn test_required_group_name() -> &'static str {
        "test-direct-group"
    }

    async fn upsert_test_group_binding(
        pool: &SqlitePool,
        group_name: &str,
        bound_proxy_keys: Vec<String>,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        let bound_proxy_keys_json =
            encode_group_bound_proxy_keys_json(&bound_proxy_keys).expect("encode test bindings");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name, note, bound_proxy_keys_json, created_at, updated_at
            ) VALUES (?1, '', ?2, ?3, ?3)
            ON CONFLICT(group_name) DO UPDATE SET
                bound_proxy_keys_json = excluded.bound_proxy_keys_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(group_name)
        .bind(bound_proxy_keys_json)
        .bind(&now_iso)
        .execute(pool)
        .await
        .expect("upsert test group binding");
    }

    async fn ensure_test_group_binding(pool: &SqlitePool, group_name: &str) {
        upsert_test_group_binding(pool, group_name, test_required_group_bound_proxy_keys()).await;
    }

    async fn set_test_account_group_name(
        pool: &SqlitePool,
        account_id: i64,
        group_name: Option<&str>,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(group_name)
        .bind(&now_iso)
        .execute(pool)
        .await
        .expect("set test account group name");
    }

    async fn test_app_state_with_usage_base(base_url: &str) -> Arc<AppState> {
        test_app_state_with_usage_base_and_parallelism(
            base_url,
            DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
        )
        .await
    }

    async fn test_app_state_with_usage_base_and_parallelism(
        base_url: &str,
        maintenance_parallelism: usize,
    ) -> Arc<AppState> {
        test_app_state_with_upstream_endpoints_and_parallelism(
            base_url,
            DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER,
            "codex-vibe-monitor/test",
            maintenance_parallelism,
        )
        .await
    }

    async fn test_app_state_with_usage_and_oauth_base(
        usage_base_url: &str,
        oauth_issuer: &str,
    ) -> Arc<AppState> {
        test_app_state_with_upstream_endpoints_and_parallelism(
            usage_base_url,
            oauth_issuer,
            UPSTREAM_USAGE_BROWSER_USER_AGENT,
            DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
        )
        .await
    }

    async fn test_app_state_with_upstream_endpoints_and_parallelism(
        usage_base_url: &str,
        oauth_issuer: &str,
        user_agent: &str,
        maintenance_parallelism: usize,
    ) -> Arc<AppState> {
        let mut config = usage_snapshot_test_config(usage_base_url, user_agent);
        config.upstream_accounts_oauth_issuer =
            Url::parse(oauth_issuer).expect("valid oauth issuer");
        test_app_state_with_config_and_parallelism(config, maintenance_parallelism).await
    }

    async fn test_app_state_with_config_and_parallelism(
        config: AppConfig,
        maintenance_parallelism: usize,
    ) -> Arc<AppState> {
        let http_clients = HttpClients::build(&config).expect("build http clients");
        let (broadcaster, _) = broadcast::channel(8);
        Arc::new(AppState {
            config,
            pool: test_pool().await,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache {
                summaries: HashMap::new(),
                quota: None,
            })),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
            startup_ready: Arc::new(AtomicBool::new(true)),
            shutdown: CancellationToken::new(),
            semaphore: Arc::new(Semaphore::new(4)),
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
                ForwardProxySettings::default(),
                Vec::new(),
            ))),
            xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
                "xray".to_string(),
                PathBuf::from("target/xray-supervisor-tests"),
            ))),
            forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
            prompt_cache_conversation_cache: Arc::new(Mutex::new(
                PromptCacheConversationsCacheState {
                    entries: HashMap::new(),
                    in_flight: HashMap::new(),
                    generation: 0,
                },
            )),
            maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
            pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
            pool_group_429_retry_delay_override: None,
            hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
            upstream_accounts: Arc::new(
                UpstreamAccountsRuntime::test_instance_with_maintenance_parallelism(
                    maintenance_parallelism,
                ),
            ),
        })
    }

    async fn ensure_window_actual_usage_test_tables(pool: &SqlitePool) {
        sqlx::query(&codex_invocations_create_sql("codex_invocations"))
            .execute(pool)
            .await
            .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS archive_batches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dataset TEXT NOT NULL,
                month_key TEXT NOT NULL,
                day_key TEXT,
                part_key TEXT,
                file_path TEXT NOT NULL,
                status TEXT NOT NULL,
                coverage_start_at TEXT,
                coverage_end_at TEXT,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("create archive_batches table");
    }

    fn shanghai_local_iso(timestamp: DateTime<Utc>) -> String {
        format_naive(timestamp.with_timezone(&Shanghai).naive_local())
    }

    async fn insert_window_actual_usage_invocation(
        pool: &SqlitePool,
        account_id: i64,
        occurred_at: &str,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        total_tokens: Option<i64>,
        cost: Option<f64>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                status,
                payload,
                raw_response,
                created_at
            ) VALUES (
                ?1,
                ?2,
                'test',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                'completed',
                ?8,
                '{}',
                ?2
            )
            "#,
        )
        .bind(format!("invoke-{}", random_base36(10).expect("invoke id")))
        .bind(occurred_at)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_input_tokens)
        .bind(total_tokens)
        .bind(cost)
        .bind(json!({ "upstreamAccountId": account_id }).to_string())
        .execute(pool)
        .await
        .expect("insert codex_invocations row");
    }

    async fn seed_window_actual_usage_archive_batch(
        pool: &SqlitePool,
        archive_dir: &Path,
        batch_name: &str,
        rows: &[(
            i64,
            String,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<f64>,
        )],
    ) -> PathBuf {
        std::fs::create_dir_all(archive_dir).expect("create archive dir");
        let archive_db_path = archive_dir.join(format!("{batch_name}.sqlite"));
        let archive_gzip_path = archive_dir.join(format!("{batch_name}.sqlite.gz"));
        let _ = std::fs::remove_file(&archive_db_path);
        let _ = std::fs::remove_file(&archive_gzip_path);
        std::fs::File::create(&archive_db_path).expect("create archive sqlite");

        let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
            .await
            .expect("open archive sqlite");
        let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
        sqlx::query(&create_sql)
            .execute(&archive_pool)
            .await
            .expect("create archive codex_invocations");

        for (index, row) in rows.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    id,
                    invoke_id,
                    occurred_at,
                    source,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_tokens,
                    cost,
                    status,
                    payload,
                    raw_response,
                    created_at
                ) VALUES (
                    ?1,
                    ?2,
                    ?3,
                    'test',
                    ?4,
                    ?5,
                    ?6,
                    ?7,
                    ?8,
                    'completed',
                    ?9,
                    '{}',
                    ?3
                )
                "#,
            )
            .bind(index as i64 + 1)
            .bind(format!(
                "archived-invoke-{}",
                random_base36(10).expect("archive invoke id")
            ))
            .bind(&row.1)
            .bind(row.2)
            .bind(row.3)
            .bind(row.4)
            .bind(row.5)
            .bind(row.6)
            .bind(json!({ "upstreamAccountId": row.0 }).to_string())
            .execute(&archive_pool)
            .await
            .expect("insert archive codex_invocations row");
        }

        archive_pool.close().await;
        deflate_sqlite_file_to_gzip(&archive_db_path, &archive_gzip_path)
            .expect("compress archive sqlite");

        let coverage_start_at = rows
            .iter()
            .map(|row| row.1.as_str())
            .min()
            .expect("archive coverage start");
        let coverage_end_at = rows
            .iter()
            .map(|row| row.1.as_str())
            .max()
            .expect("archive coverage end");
        let month_key = &coverage_start_at[..7];
        let day_key = &coverage_start_at[..10];

        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                day_key,
                part_key,
                file_path,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            ) VALUES (
                'codex_invocations',
                ?1,
                ?2,
                'part-000',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7
            )
            "#,
        )
        .bind(month_key)
        .bind(day_key)
        .bind(format!("{batch_name}.sqlite.gz"))
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(coverage_start_at)
        .bind(coverage_end_at)
        .bind(coverage_end_at)
        .execute(pool)
        .await
        .expect("insert archive batch manifest");

        archive_gzip_path
    }

    fn assert_cost_close(actual: f64, expected: f64) {
        let diff = (actual - expected).abs();
        assert!(
            diff < 1e-9,
            "expected {expected}, got {actual}, diff={diff}"
        );
    }

    #[derive(Clone)]
    struct MoeMailStubState {
        email_domains: String,
        emails: Arc<Mutex<Vec<(String, String, Option<String>)>>>,
        generated_requests: Arc<Mutex<Vec<(String, String)>>>,
        deleted_ids: Arc<Mutex<Vec<String>>>,
        next_generated_id: Arc<AtomicUsize>,
    }

    struct MoeMailTestHarness {
        state: Arc<AppState>,
        stub: MoeMailStubState,
        server: tokio::task::JoinHandle<()>,
    }

    impl MoeMailTestHarness {
        fn abort(self) {
            self.server.abort();
        }
    }

    async fn spawn_moemail_test_harness(
        email_domains: &str,
        emails: Vec<(String, String, Option<String>)>,
    ) -> MoeMailTestHarness {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GenerateRequest {
            name: String,
            domain: String,
        }

        async fn config_handler(
            State(state): State<MoeMailStubState>,
        ) -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "defaultRole": "DUKE",
                "emailDomains": state.email_domains,
                "maxEmails": "20",
            }))
        }

        async fn list_emails_handler(
            State(state): State<MoeMailStubState>,
        ) -> axum::Json<serde_json::Value> {
            let emails = state.emails.lock().await.clone();
            axum::Json(json!({
                "emails": emails.into_iter().map(|(id, address, expires_at)| json!({
                    "id": id,
                    "address": address,
                    "expiresAt": expires_at,
                })).collect::<Vec<_>>(),
                "nextCursor": null,
            }))
        }

        async fn create_email_handler(
            State(state): State<MoeMailStubState>,
            axum::Json(payload): axum::Json<GenerateRequest>,
        ) -> axum::Json<serde_json::Value> {
            let index = state
                .next_generated_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            let email = format!("{}@{}", payload.name, payload.domain);
            let id = format!("generated_{index}");
            state
                .generated_requests
                .lock()
                .await
                .push((payload.name.clone(), payload.domain.clone()));
            state
                .emails
                .lock()
                .await
                .push((id.clone(), email.clone(), None));
            axum::Json(json!({ "id": id, "email": email }))
        }

        async fn messages_handler() -> axum::Json<serde_json::Value> {
            axum::Json(json!({ "messages": [] }))
        }

        async fn delete_email_handler(
            State(state): State<MoeMailStubState>,
            axum::extract::Path(email_id): axum::extract::Path<String>,
        ) -> axum::Json<serde_json::Value> {
            state.deleted_ids.lock().await.push(email_id.clone());
            state
                .emails
                .lock()
                .await
                .retain(|(existing_id, _, _)| existing_id != &email_id);
            axum::Json(json!({ "success": true }))
        }

        let stub = MoeMailStubState {
            email_domains: email_domains.to_string(),
            emails: Arc::new(Mutex::new(emails)),
            generated_requests: Arc::new(Mutex::new(Vec::new())),
            deleted_ids: Arc::new(Mutex::new(Vec::new())),
            next_generated_id: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/api/config", get(config_handler))
            .route("/api/emails", get(list_emails_handler))
            .route("/api/emails/generate", post(create_email_handler))
            .route(
                "/api/emails/:email_id",
                get(messages_handler).delete(delete_email_handler),
            )
            .with_state(stub.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind moemail test listener");
        let addr = listener.local_addr().expect("moemail listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve moemail test app");
        });

        let mut config = usage_snapshot_test_config(
            "https://chatgpt.com/backend-api",
            "codex-vibe-monitor/test",
        );
        config.upstream_accounts_moemail = Some(UpstreamAccountsMoeMailConfig {
            base_url: Url::parse(&format!("http://{addr}")).expect("valid moemail test url"),
            api_key: "test-moemail-key".to_string(),
            default_domain: "mail-tw.707079.xyz".to_string(),
        });
        let http_clients = HttpClients::build(&config).expect("build http clients");
        let (broadcaster, _) = broadcast::channel(8);
        let state = Arc::new(AppState {
            config,
            pool: test_pool().await,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache {
                summaries: HashMap::new(),
                quota: None,
            })),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
            startup_ready: Arc::new(AtomicBool::new(true)),
            shutdown: CancellationToken::new(),
            semaphore: Arc::new(Semaphore::new(4)),
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
                ForwardProxySettings::default(),
                Vec::new(),
            ))),
            xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
                "xray".to_string(),
                PathBuf::from("target/xray-supervisor-tests"),
            ))),
            forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
            prompt_cache_conversation_cache: Arc::new(Mutex::new(
                PromptCacheConversationsCacheState {
                    entries: HashMap::new(),
                    in_flight: HashMap::new(),
                    generation: 0,
                },
            )),
            maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
            pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
            pool_group_429_retry_delay_override: None,
            hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
            upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
        });

        MoeMailTestHarness {
            state,
            stub,
            server,
        }
    }

    fn test_claims_with_plan_type(
        email: &str,
        chatgpt_account_id: Option<&str>,
        chatgpt_user_id: Option<&str>,
        plan_type: Option<&str>,
    ) -> ChatgptJwtClaims {
        ChatgptJwtClaims {
            email: Some(email.to_string()),
            chatgpt_plan_type: plan_type.map(str::to_string),
            chatgpt_user_id: chatgpt_user_id.map(str::to_string),
            chatgpt_account_id: chatgpt_account_id.map(str::to_string),
        }
    }

    fn test_claims(
        email: &str,
        chatgpt_account_id: Option<&str>,
        chatgpt_user_id: Option<&str>,
    ) -> ChatgptJwtClaims {
        test_claims_with_plan_type(email, chatgpt_account_id, chatgpt_user_id, Some("team"))
    }

    fn test_id_token(
        email: &str,
        chatgpt_account_id: Option<&str>,
        chatgpt_user_id: Option<&str>,
        plan_type: Option<&str>,
    ) -> String {
        test_jwt_token(json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": plan_type,
                "chatgpt_user_id": chatgpt_user_id,
                "chatgpt_account_id": chatgpt_account_id,
            }
        }))
    }

    fn test_jwt_token(payload: serde_json::Value) -> String {
        let encoded = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{encoded}.{body}.{encoded}")
    }

    fn test_tag_routing_rule() -> TagRoutingRule {
        TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: true,
            allow_cut_in: true,
        }
    }

    async fn insert_test_oauth_mailbox_session(
        pool: &SqlitePool,
        session_id: &str,
        email_address: &str,
        source: &str,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        let expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(1));
        let domain = email_address
            .split('@')
            .nth(1)
            .unwrap_or("mail-tw.707079.xyz");
        sqlx::query(
            r#"
            INSERT INTO pool_oauth_mailbox_sessions (
                session_id, remote_email_id, email_address, email_domain, mailbox_source,
                latest_code_value, latest_code_source, latest_code_updated_at,
                invite_subject, invite_copy_value, invite_copy_label, invite_updated_at,
                invited, last_message_id, created_at, updated_at, expires_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                NULL, NULL, NULL,
                NULL, NULL, NULL, NULL,
                0, NULL, ?6, ?6, ?7
            )
            "#,
        )
        .bind(session_id)
        .bind(format!("remote-{session_id}"))
        .bind(email_address)
        .bind(domain)
        .bind(source)
        .bind(&now_iso)
        .bind(&expires_at)
        .execute(pool)
        .await
        .expect("insert oauth mailbox session");
    }

    async fn insert_api_key_account(pool: &SqlitePool, display_name: &str) -> i64 {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
                chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
                local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, NULL, NULL, ?4, 1, NULL, NULL,
                NULL, NULL, ?5, ?6, NULL,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?7, ?7
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("sk-test")
        .bind("encrypted")
        .bind(&now_iso)
        .fetch_one(pool)
        .await
        .expect("insert api key account")
    }

    async fn insert_oauth_account(pool: &SqlitePool, display_name: &str) -> i64 {
        ensure_test_group_binding(pool, test_required_group_name()).await;
        let now_iso = format_utc_iso(Utc::now());
        let token_expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(30));
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
                chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
                local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5, 1, ?6, ?7,
                ?8, ?9, NULL, ?10, ?11,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?12, ?12
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(test_required_group_name())
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("oauth@example.com")
        .bind("org_test")
        .bind("user_test")
        .bind("team")
        .bind("encrypted")
        .bind(&token_expires_at)
        .bind(&now_iso)
        .fetch_one(pool)
        .await
        .expect("insert oauth account")
    }

    async fn insert_syncable_oauth_account(
        pool: &SqlitePool,
        crypto_key: &[u8; 32],
        display_name: &str,
        email: &str,
        account_id: &str,
        user_id: &str,
    ) -> i64 {
        ensure_test_group_binding(pool, test_required_group_name()).await;
        let now_iso = format_utc_iso(Utc::now());
        let token_expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(30));
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "access-token".to_string(),
                refresh_token: "refresh-token".to_string(),
                id_token: test_id_token(email, Some(account_id), Some(user_id), Some("team")),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
                chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
                local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5, 1, ?6, ?7,
                ?8, ?9, NULL, ?10, ?11,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?12, ?12
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(test_required_group_name())
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(email)
        .bind(account_id)
        .bind(user_id)
        .bind("team")
        .bind(encrypted_credentials)
        .bind(&token_expires_at)
        .bind(&now_iso)
        .fetch_one(pool)
        .await
        .expect("insert syncable oauth account")
    }

    async fn spawn_usage_snapshot_server(
        status: StatusCode,
        body: serde_json::Value,
    ) -> (String, JoinHandle<()>) {
        async fn handler(
            State((status, body)): State<(StatusCode, Arc<String>)>,
        ) -> (StatusCode, String) {
            (status, (*body).clone())
        }

        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state((status, Arc::new(body.to_string())));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage snapshot server");
        let addr = listener.local_addr().expect("usage snapshot server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage snapshot server");
        });

        (format!("http://{addr}/backend-api"), server)
    }

    #[derive(Clone)]
    struct SequencedOauthSyncServerState {
        usage_responses: Arc<Mutex<std::collections::VecDeque<(StatusCode, String)>>>,
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
        token_response: Arc<String>,
    }

    async fn spawn_sequenced_oauth_sync_server(
        usage_responses: Vec<(StatusCode, serde_json::Value)>,
        token_response: serde_json::Value,
    ) -> (
        String,
        String,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        JoinHandle<()>,
    ) {
        async fn usage_handler(
            State(state): State<SequencedOauthSyncServerState>,
        ) -> (StatusCode, String) {
            state.usage_requests.fetch_add(1, Ordering::SeqCst);
            let mut responses = state.usage_responses.lock().await;
            responses.pop_front().unwrap_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({
                        "error": {
                            "message": "unexpected extra usage request"
                        }
                    })
                    .to_string(),
                )
            })
        }

        async fn token_handler(
            State(state): State<SequencedOauthSyncServerState>,
        ) -> (StatusCode, String) {
            state.token_requests.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, (*state.token_response).clone())
        }

        let usage_responses = usage_responses
            .into_iter()
            .map(|(status, body)| (status, body.to_string()))
            .collect::<std::collections::VecDeque<_>>();
        let usage_requests = Arc::new(AtomicUsize::new(0));
        let token_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(usage_handler))
            .route("/oauth/token", post(token_handler))
            .with_state(SequencedOauthSyncServerState {
                usage_responses: Arc::new(Mutex::new(usage_responses)),
                usage_requests: usage_requests.clone(),
                token_requests: token_requests.clone(),
                token_response: Arc::new(token_response.to_string()),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind sequenced oauth sync server");
        let addr = listener
            .local_addr()
            .expect("sequenced oauth sync server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve sequenced oauth sync server");
        });
        let origin = format!("http://{addr}");

        (
            format!("{origin}/backend-api"),
            origin,
            usage_requests,
            token_requests,
            server,
        )
    }

    #[derive(Clone)]
    struct BlockingUsageServerState {
        started: Arc<AtomicBool>,
        release: Arc<Notify>,
        requests: Arc<AtomicUsize>,
    }

    async fn spawn_blocking_usage_server() -> (
        String,
        Arc<AtomicBool>,
        Arc<Notify>,
        Arc<AtomicUsize>,
        JoinHandle<()>,
    ) {
        async fn handler(State(state): State<BlockingUsageServerState>) -> (StatusCode, String) {
            state.requests.fetch_add(1, Ordering::SeqCst);
            state.started.store(true, Ordering::SeqCst);
            state.release.notified().await;
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        }

        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(Notify::new());
        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(BlockingUsageServerState {
                started: started.clone(),
                release: release.clone(),
                requests: requests.clone(),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind blocking usage server");
        let addr = listener.local_addr().expect("blocking usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve blocking usage server");
        });

        (
            format!("http://{addr}/backend-api"),
            started,
            release,
            requests,
            server,
        )
    }

    async fn wait_for_atomic_true(flag: &AtomicBool) {
        timeout(Duration::from_secs(1), async {
            while !flag.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("flag should become true");
    }

    async fn wait_for_atomic_usize(flag: &AtomicUsize, expected: usize) {
        timeout(Duration::from_secs(1), async {
            while flag.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("counter should reach expected value");
    }

    #[tokio::test]
    async fn maintenance_pass_dispatches_without_waiting_for_sync_completion() {
        let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Queued Maintenance OAuth",
            "queued-maintenance@example.com",
            "org_queued_maintenance",
            "user_queued_maintenance",
        )
        .await;

        let started_at = std::time::Instant::now();
        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("maintenance pass should dispatch");
        assert!(
            started_at.elapsed() < Duration::from_secs(1),
            "maintenance pass should return after dispatching work"
        );

        wait_for_atomic_true(started.as_ref()).await;
        release.notify_waiters();
        timeout(Duration::from_secs(1), async {
            while requests.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued maintenance request should complete");
        server.abort();
    }

    #[tokio::test]
    async fn drain_background_tasks_waits_for_queued_maintenance_syncs() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Drain Maintenance OAuth",
            "drain-maintenance@example.com",
            "org_drain_maintenance",
            "user_drain_maintenance",
        )
        .await;

        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("maintenance pass should dispatch");
        wait_for_atomic_true(started.as_ref()).await;

        let mut drain_task = tokio::spawn({
            let runtime = state.upstream_accounts.clone();
            async move {
                runtime.drain_background_tasks().await;
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut drain_task)
                .await
                .is_err(),
            "drain should wait for queued maintenance tasks"
        );

        release.notify_waiters();
        drain_task.await.expect("drain join should succeed");
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_dispatch_respects_parallelism_limit() {
        async fn handler(
            State((requests, release)): State<(Arc<AtomicUsize>, Arc<Semaphore>)>,
        ) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            let _permit = release
                .acquire()
                .await
                .expect("test release semaphore should stay open");
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        }

        let requests = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state((requests.clone(), release.clone()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind bounded usage server");
        let addr = listener.local_addr().expect("bounded usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve bounded usage server");
        });

        let state = test_app_state_with_usage_base_and_parallelism(
            &format!("http://{addr}/backend-api"),
            1,
        )
        .await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Bounded Maintenance A",
            "bounded-a@example.com",
            "org_bounded_a",
            "user_bounded_a",
        )
        .await;
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Bounded Maintenance B",
            "bounded-b@example.com",
            "org_bounded_b",
            "user_bounded_b",
        )
        .await;

        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("dispatch maintenance pass");
        wait_for_atomic_usize(requests.as_ref(), 1).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "only one maintenance sync should reach the upstream at a time"
        );

        release.add_permits(1);
        wait_for_atomic_usize(requests.as_ref(), 2).await;
        release.add_permits(1);
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_sync_does_not_block_unrelated_account_updates() {
        let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let maintenance_account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Maintenance OAuth",
            "maintenance@example.com",
            "org_maintenance",
            "user_maintenance",
        )
        .await;
        let updated_account_id = insert_api_key_account(&state.pool, "Unrelated API Key").await;

        let maintenance_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_maintenance_sync(state.clone(), maintenance_account_id)
                    .await
            }
        });
        wait_for_atomic_true(started.as_ref()).await;

        let started_at = std::time::Instant::now();
        state
            .upstream_accounts
            .account_ops
            .run_update_account(
                state.clone(),
                updated_account_id,
                UpdateUpstreamAccountRequest {
                    display_name: None,
                    group_name: None,
                    group_bound_proxy_keys: None,
                    note: Some("updated while maintenance runs".to_string()),
                    group_note: None,
                    upstream_base_url: OptionalField::Missing,
                    enabled: Some(false),
                    is_mother: None,
                    api_key: None,
                    local_primary_limit: None,
                    local_secondary_limit: None,
                    local_limit_unit: None,
                    tag_ids: None,
                },
            )
            .await
            .expect("update unrelated account");
        assert!(
            started_at.elapsed() < Duration::from_secs(1),
            "unrelated account update should not wait for maintenance"
        );

        let updated_row = load_upstream_account_row(&state.pool, updated_account_id)
            .await
            .expect("load updated account")
            .expect("updated account exists");
        assert_eq!(updated_row.enabled, 0);
        assert_eq!(
            updated_row.note.as_deref(),
            Some("updated while maintenance runs")
        );

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn persist_imported_oauth_waits_for_inflight_maintenance() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Imported OAuth",
            "imported@example.com",
            "org_imported",
            "user_imported",
        )
        .await;

        let maintenance_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_maintenance_sync(state.clone(), account_id)
                    .await
            }
        });
        wait_for_atomic_true(started.as_ref()).await;

        let probe = ImportedOauthProbeOutcome {
            token_expires_at: format_utc_iso(Utc::now() + ChronoDuration::days(30)),
            credentials: StoredOauthCredentials {
                access_token: "imported-access-token".to_string(),
                refresh_token: "imported-refresh-token".to_string(),
                id_token: test_id_token(
                    "imported@example.com",
                    Some("org_imported"),
                    Some("user_imported"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            },
            claims: test_claims(
                "imported@example.com",
                Some("org_imported"),
                Some("user_imported"),
            ),
            usage_snapshot: None,
            exhausted: false,
            usage_snapshot_warning: Some("usage snapshot unavailable".to_string()),
        };

        let mut import_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_persist_imported_oauth(state.clone(), account_id, probe)
                    .await
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut import_task)
                .await
                .is_err(),
            "post-import updates should queue behind same-account maintenance"
        );

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        assert_eq!(
            import_task
                .await
                .expect("import join")
                .expect("persist imported oauth"),
            Some("usage snapshot unavailable".to_string())
        );
        server.abort();
    }

    #[tokio::test]
    async fn same_account_updates_wait_for_inflight_maintenance() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Serialized OAuth",
            "serialized@example.com",
            "org_serialized",
            "user_serialized",
        )
        .await;

        let maintenance_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_maintenance_sync(state.clone(), account_id)
                    .await
            }
        });
        wait_for_atomic_true(started.as_ref()).await;

        let mut update_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_update_account(
                        state.clone(),
                        account_id,
                        UpdateUpstreamAccountRequest {
                            display_name: None,
                            group_name: None,
                            group_bound_proxy_keys: None,
                            note: Some("queued note".to_string()),
                            group_note: None,
                            upstream_base_url: OptionalField::Missing,
                            enabled: None,
                            is_mother: None,
                            api_key: None,
                            local_primary_limit: None,
                            local_secondary_limit: None,
                            local_limit_unit: None,
                            tag_ids: None,
                        },
                    )
                    .await
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut update_task)
                .await
                .is_err(),
            "same-account update should queue behind maintenance"
        );

        let row_during_maintenance = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load row during maintenance")
            .expect("row exists");
        assert_eq!(row_during_maintenance.note, None);

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        update_task
            .await
            .expect("update join")
            .expect("update result");

        let updated_row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        assert_eq!(updated_row.note.as_deref(), Some("queued note"));
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_sync_deduplicates_same_account_work() {
        let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Deduped OAuth",
            "deduped@example.com",
            "org_deduped",
            "user_deduped",
        )
        .await;

        let maintenance_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_maintenance_sync(state.clone(), account_id)
                    .await
            }
        });
        wait_for_atomic_true(started.as_ref()).await;

        let second = state
            .upstream_accounts
            .account_ops
            .run_maintenance_sync(state.clone(), account_id)
            .await
            .expect("second maintenance result");
        assert_eq!(second, MaintenanceDispatchOutcome::Deduped);

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn queued_maintenance_sync_revalidates_due_window_before_execution() {
        async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        },
                        "secondaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 10080,
                            "resetsAt": 1771927200
                        }
                    }
                })
                .to_string(),
            )
        }

        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(requests.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage server");
        let addr = listener.local_addr().expect("usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage server");
        });

        let state = test_app_state_with_usage_base_and_parallelism(
            &format!("http://{addr}/backend-api"),
            1,
        )
        .await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Queued Revalidation OAuth",
            "queued-revalidation@example.com",
            "org_queued_revalidation",
            "user_queued_revalidation",
        )
        .await;
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            "2026-03-23T11:00:00Z",
            Some(12.0),
            Some(10.0),
        )
        .await;

        let due_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(10));
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&due_at)
        .execute(&state.pool)
        .await
        .expect("seed due sync time");

        let held_slot = state
            .upstream_accounts
            .account_ops
            .maintenance_slots
            .clone()
            .acquire_owned()
            .await
            .expect("hold maintenance slot");
        assert_eq!(
            state
                .upstream_accounts
                .account_ops
                .dispatch_maintenance_sync(
                    state.clone(),
                    MaintenanceDispatchPlan {
                        account_id,
                        tier: MaintenanceTier::Priority,
                        sync_interval_secs: 300,
                    },
                )
                .expect("queue maintenance plan"),
            MaintenanceQueueOutcome::Queued
        );

        let fresh_sync_at = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&fresh_sync_at)
        .execute(&state.pool)
        .await
        .expect("refresh sync time before queued maintenance executes");

        drop(held_slot);
        state.upstream_accounts.drain_background_tasks().await;

        assert_eq!(
            requests.load(Ordering::SeqCst),
            0,
            "queued maintenance should skip once a newer sync makes the plan stale"
        );
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_dedupe_flag_resets_after_panicking_job() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = 777_i64;

        let first: Result<AccountSubmitOutcome<()>, AccountCommandDispatchError<anyhow::Error>> =
            state
                .upstream_accounts
                .account_ops
                .submit_command(
                    state.clone(),
                    account_id,
                    AccountCommand::MaintenanceSync,
                    true,
                    |_state, _id| async move {
                        let _: Result<(), anyhow::Error> = Ok(());
                        panic!("simulated maintenance panic");
                    },
                )
                .await;
        assert!(matches!(
            first,
            Err(AccountCommandDispatchError::ActorUnavailable(
                AccountCommand::MaintenanceSync
            ))
        ));

        let second = state
            .upstream_accounts
            .account_ops
            .submit_command(
                state.clone(),
                account_id,
                AccountCommand::MaintenanceSync,
                true,
                |_state, _id| async move { Result::<(), anyhow::Error>::Ok(()) },
            )
            .await
            .expect("second maintenance command should be accepted");
        assert!(matches!(second, AccountSubmitOutcome::Completed(())));
        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);
    }

    #[tokio::test]
    async fn ensure_upstream_accounts_schema_seeds_pool_routing_settings_for_new_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("ensure schema");

        let config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
        let row = load_pool_routing_settings_seeded(&pool, &config)
            .await
            .expect("load seeded routing settings");

        assert_eq!(row.masked_api_key, None);
        assert_eq!(row.primary_sync_interval_secs, None);
        assert_eq!(row.secondary_sync_interval_secs, None);
        assert_eq!(row.priority_available_account_cap, None);
        assert_eq!(row.responses_first_byte_timeout_secs, None);
        assert_eq!(row.compact_first_byte_timeout_secs, None);
        assert_eq!(row.responses_stream_timeout_secs, None);
        assert_eq!(row.compact_stream_timeout_secs, None);
        assert_eq!(row.default_first_byte_timeout_secs, None);
        assert_eq!(row.upstream_handshake_timeout_secs, None);
        assert_eq!(row.request_read_timeout_secs, None);
    }

    #[tokio::test]
    async fn ensure_upstream_accounts_schema_upgrades_legacy_pool_routing_settings_before_seed() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        sqlx::query(
            r#"
            CREATE TABLE pool_routing_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                encrypted_api_key TEXT,
                masked_api_key TEXT,
                primary_sync_interval_secs INTEGER,
                secondary_sync_interval_secs INTEGER,
                priority_available_account_cap INTEGER,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy pool_routing_settings");
        sqlx::query(
            r#"
            INSERT INTO pool_routing_settings (
                id,
                encrypted_api_key,
                masked_api_key,
                primary_sync_interval_secs,
                secondary_sync_interval_secs,
                priority_available_account_cap,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
            "#,
        )
        .bind(POOL_SETTINGS_SINGLETON_ID)
        .bind("legacy-ciphertext")
        .bind("sk-legacy")
        .bind(300_i64)
        .bind(2400_i64)
        .bind(99_i64)
        .execute(&pool)
        .await
        .expect("insert legacy pool routing row");

        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("upgrade schema");

        let columns = sqlx::query("PRAGMA table_info('pool_routing_settings')")
            .fetch_all(&pool)
            .await
            .expect("load table info")
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect::<std::collections::HashSet<_>>();
        for column in [
            "responses_first_byte_timeout_secs",
            "compact_first_byte_timeout_secs",
            "responses_stream_timeout_secs",
            "compact_stream_timeout_secs",
            "default_first_byte_timeout_secs",
            "upstream_handshake_timeout_secs",
            "request_read_timeout_secs",
        ] {
            assert!(
                columns.contains(column),
                "expected upgraded schema to contain {column}"
            );
        }

        let config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
        let row = load_pool_routing_settings_seeded(&pool, &config)
            .await
            .expect("load upgraded routing settings");
        assert_eq!(row.encrypted_api_key.as_deref(), Some("legacy-ciphertext"));
        assert_eq!(row.masked_api_key.as_deref(), Some("sk-legacy"));
        assert_eq!(row.primary_sync_interval_secs, Some(300));
        assert_eq!(row.secondary_sync_interval_secs, Some(2400));
        assert_eq!(row.priority_available_account_cap, Some(99));
        assert_eq!(row.responses_first_byte_timeout_secs, None);
        assert_eq!(row.compact_first_byte_timeout_secs, None);
        assert_eq!(row.responses_stream_timeout_secs, None);
        assert_eq!(row.compact_stream_timeout_secs, None);
        assert_eq!(row.default_first_byte_timeout_secs, None);
        assert_eq!(row.upstream_handshake_timeout_secs, None);
        assert_eq!(row.request_read_timeout_secs, None);

        let resolved = resolve_pool_routing_timeouts(&pool, &config)
            .await
            .expect("resolve routing timeouts");
        let defaults = pool_routing_timeouts_from_config(&config);
        assert_eq!(
            resolved.responses_first_byte_timeout,
            defaults.responses_first_byte_timeout
        );
        assert_eq!(
            resolved.compact_first_byte_timeout,
            defaults.compact_first_byte_timeout
        );
        assert_eq!(
            resolved.responses_stream_timeout,
            defaults.responses_stream_timeout
        );
        assert_eq!(
            resolved.compact_stream_timeout,
            defaults.compact_stream_timeout
        );
        assert_eq!(
            resolved.default_first_byte_timeout,
            defaults.default_first_byte_timeout
        );
        assert_eq!(resolved.default_send_timeout, defaults.default_send_timeout);
        assert_eq!(resolved.request_read_timeout, defaults.request_read_timeout);
    }

    #[tokio::test]
    async fn update_pool_routing_settings_allows_maintenance_only_patch() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        save_pool_routing_api_key(&state.pool, crypto_key, "pool-live-key")
            .await
            .expect("seed pool api key");

        let payload: UpdatePoolRoutingSettingsRequest = serde_json::from_value(json!({
            "maintenance": {
                "secondarySyncIntervalSecs": 2400
            }
        }))
        .expect("deserialize maintenance patch");
        let Json(response) =
            update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
                .await
                .expect("update routing settings");
        let expected_mask = mask_api_key("pool-live-key");

        assert!(response.api_key_configured);
        assert_eq!(
            response.masked_api_key.as_deref(),
            Some(expected_mask.as_str())
        );
        assert_eq!(response.maintenance.primary_sync_interval_secs, 300);
        assert_eq!(response.maintenance.secondary_sync_interval_secs, 2400);
        assert_eq!(response.maintenance.priority_available_account_cap, 100);

        let stored = load_pool_routing_settings(&state.pool)
            .await
            .expect("load routing settings");
        assert!(stored.encrypted_api_key.is_some());
        assert_eq!(stored.secondary_sync_interval_secs, Some(2400));
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_prioritizes_forced_accounts_and_overflow() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let refresh_lead_time = Duration::from_secs(15 * 60);

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![
                MaintenanceCandidateRow {
                    id: 1,
                    status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                    last_synced_at: Some("2026-03-23T11:40:00Z".to_string()),
                    last_error_at: None,
                    token_expires_at: Some("2026-04-23T12:00:00Z".to_string()),
                    primary_used_percent: Some(15.0),
                    secondary_used_percent: Some(10.0),
                },
                MaintenanceCandidateRow {
                    id: 2,
                    status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                    last_synced_at: Some("2026-03-23T11:20:00Z".to_string()),
                    last_error_at: None,
                    token_expires_at: Some("2026-04-23T12:00:00Z".to_string()),
                    primary_used_percent: Some(12.0),
                    secondary_used_percent: Some(22.0),
                },
                MaintenanceCandidateRow {
                    id: 3,
                    status: UPSTREAM_ACCOUNT_STATUS_ERROR.to_string(),
                    last_synced_at: None,
                    last_error_at: Some("2026-03-23T11:58:30Z".to_string()),
                    token_expires_at: Some("2026-04-23T12:00:00Z".to_string()),
                    primary_used_percent: Some(8.0),
                    secondary_used_percent: Some(8.0),
                },
                MaintenanceCandidateRow {
                    id: 4,
                    status: UPSTREAM_ACCOUNT_STATUS_ERROR.to_string(),
                    last_synced_at: None,
                    last_error_at: Some("2026-03-23T11:50:00Z".to_string()),
                    token_expires_at: Some("2026-04-23T12:00:00Z".to_string()),
                    primary_used_percent: Some(9.0),
                    secondary_used_percent: Some(9.0),
                },
                MaintenanceCandidateRow {
                    id: 5,
                    status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                    last_synced_at: Some("2026-03-23T11:50:00Z".to_string()),
                    last_error_at: None,
                    token_expires_at: Some("2026-04-23T12:00:00Z".to_string()),
                    primary_used_percent: Some(5.0),
                    secondary_used_percent: None,
                },
                MaintenanceCandidateRow {
                    id: 6,
                    status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                    last_synced_at: Some("2026-03-23T11:54:00Z".to_string()),
                    last_error_at: None,
                    token_expires_at: Some("2026-03-23T12:10:00Z".to_string()),
                    primary_used_percent: Some(4.0),
                    secondary_used_percent: Some(4.0),
                },
            ],
            settings,
            refresh_lead_time,
            now,
        );

        let plan_map = plans
            .into_iter()
            .map(|plan| (plan.account_id, (plan.tier, plan.sync_interval_secs)))
            .collect::<HashMap<_, _>>();
        assert_eq!(plan_map.get(&1), Some(&(MaintenanceTier::Priority, 300)));
        assert_eq!(plan_map.get(&2), Some(&(MaintenanceTier::Secondary, 1800)));
        assert_eq!(plan_map.get(&4), Some(&(MaintenanceTier::Priority, 300)));
        assert_eq!(plan_map.get(&5), Some(&(MaintenanceTier::Priority, 300)));
        assert_eq!(plan_map.get(&6), Some(&(MaintenanceTier::Priority, 300)));
        assert!(!plan_map.contains_key(&3));
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_keeps_refresh_due_accounts_on_primary_cadence() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 100,
        };

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![MaintenanceCandidateRow {
                id: 7,
                status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                last_synced_at: Some("2026-03-23T11:59:00Z".to_string()),
                last_error_at: None,
                token_expires_at: Some("2026-03-23T12:10:00Z".to_string()),
                primary_used_percent: Some(6.0),
                secondary_used_percent: Some(6.0),
            }],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert!(
            plans.is_empty(),
            "refresh-due accounts should stay on the configured primary cadence until the interval elapses"
        );
    }

    fn test_routing_candidate(id: i64) -> AccountRoutingCandidateRow {
        AccountRoutingCandidateRow {
            id,
            plan_type: None,
            secondary_used_percent: None,
            secondary_window_minutes: None,
            secondary_resets_at: None,
            primary_used_percent: None,
            primary_window_minutes: None,
            primary_resets_at: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            credits_has_credits: None,
            credits_unlimited: None,
            credits_balance: None,
            last_selected_at: Some("2026-03-23T11:00:00Z".to_string()),
            active_sticky_conversations: 0,
            in_flight_reservations: 0,
        }
    }

    #[test]
    fn compare_routing_candidates_prefers_short_window_that_is_about_to_reset() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
            .single()
            .expect("valid now");
        let mut short_reset_soon = test_routing_candidate(1);
        short_reset_soon.plan_type = Some("team".to_string());
        short_reset_soon.primary_used_percent = Some(70.0);
        short_reset_soon.primary_window_minutes = Some(300);
        short_reset_soon.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(5)));
        short_reset_soon.secondary_used_percent = Some(40.0);
        short_reset_soon.secondary_window_minutes = Some(7 * 24 * 60);
        short_reset_soon.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(1)));

        let mut long_only = test_routing_candidate(2);
        long_only.plan_type = Some("free".to_string());
        long_only.secondary_used_percent = Some(30.0);
        long_only.secondary_window_minutes = Some(7 * 24 * 60);
        long_only.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

        assert_eq!(
            compare_routing_candidates_at(&short_reset_soon, &long_only, now),
            std::cmp::Ordering::Less,
            "a short-window account that is about to reset should beat a lower-used long-only account whose reset is still far away",
        );
    }

    #[test]
    fn compare_routing_candidates_penalizes_far_from_reset_pressure() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
            .single()
            .expect("valid now");
        let mut stretched_team = test_routing_candidate(1);
        stretched_team.plan_type = Some("team".to_string());
        stretched_team.primary_used_percent = Some(95.0);
        stretched_team.primary_window_minutes = Some(300);
        stretched_team.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(250)));
        stretched_team.secondary_used_percent = Some(80.0);
        stretched_team.secondary_window_minutes = Some(7 * 24 * 60);
        stretched_team.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

        let mut healthier_long = test_routing_candidate(2);
        healthier_long.plan_type = Some("free".to_string());
        healthier_long.secondary_used_percent = Some(20.0);
        healthier_long.secondary_window_minutes = Some(7 * 24 * 60);
        healthier_long.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(3)));

        assert_eq!(
            compare_routing_candidates_at(&stretched_team, &healthier_long, now),
            std::cmp::Ordering::Greater,
            "near-exhausted windows with lots of time left should sort behind healthier long-window accounts",
        );
    }

    #[test]
    fn compare_routing_candidates_treats_zero_percent_single_window_as_limited() {
        let mut single_window = test_routing_candidate(1);
        single_window.primary_used_percent = Some(0.0);
        single_window.primary_window_minutes = Some(7 * 24 * 60);
        single_window.active_sticky_conversations = 2;

        let unlimited = test_routing_candidate(2);

        assert_eq!(
            compare_routing_candidates(&single_window, &unlimited),
            std::cmp::Ordering::Greater,
            "a single remote window sample, even at 0%, should still participate in the tighter long-window load caps",
        );
    }

    #[test]
    fn candidate_capacity_profile_tightens_for_long_only_accounts() {
        let mut long_only = test_routing_candidate(1);
        long_only.secondary_used_percent = Some(10.0);
        long_only.secondary_window_minutes = Some(7 * 24 * 60);
        let mut short_window = test_routing_candidate(2);
        short_window.primary_used_percent = Some(10.0);
        short_window.primary_window_minutes = Some(300);

        let long_only_capacity = long_only.capacity_profile();
        let short_window_capacity = short_window.capacity_profile();

        assert_eq!(long_only_capacity.soft_limit, 1);
        assert_eq!(long_only_capacity.hard_cap, 2);
        assert_eq!(short_window_capacity.soft_limit, 2);
        assert_eq!(short_window_capacity.hard_cap, 3);
    }

    #[test]
    fn candidate_capacity_profile_preserves_legacy_limit_signals_without_window_metadata() {
        let mut legacy_long_only = test_routing_candidate(1);
        legacy_long_only.secondary_used_percent = Some(10.0);

        let mut locally_limited = test_routing_candidate(2);
        locally_limited.local_secondary_limit = Some(100.0);

        let legacy_capacity = legacy_long_only.capacity_profile();
        let local_capacity = locally_limited.capacity_profile();

        assert_eq!(legacy_capacity.soft_limit, 1);
        assert_eq!(legacy_capacity.hard_cap, 2);
        assert_eq!(local_capacity.soft_limit, 1);
        assert_eq!(local_capacity.hard_cap, 2);
    }

    #[tokio::test]
    async fn maintenance_pass_skips_secondary_overflow_accounts_until_secondary_interval() {
        async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        },
                        "secondaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 10080,
                            "resetsAt": 1771927200
                        }
                    }
                })
                .to_string(),
            )
        }

        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(requests.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage server");
        let addr = listener.local_addr().expect("usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage server");
        });

        let state = test_app_state_with_usage_base(&format!("http://{addr}/backend-api")).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let priority_account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Priority OAuth",
            "priority@example.com",
            "org_priority",
            "user_priority",
        )
        .await;
        let secondary_account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Secondary OAuth",
            "secondary@example.com",
            "org_secondary",
            "user_secondary",
        )
        .await;
        save_pool_routing_maintenance_settings(
            &state.pool,
            PoolRoutingMaintenanceSettings {
                primary_sync_interval_secs: 300,
                secondary_sync_interval_secs: 1800,
                priority_available_account_cap: 1,
            },
        )
        .await
        .expect("save maintenance settings");
        insert_limit_sample_with_usage(
            &state.pool,
            priority_account_id,
            "2026-03-23T11:00:00Z",
            Some(12.0),
            Some(10.0),
        )
        .await;
        insert_limit_sample_with_usage(
            &state.pool,
            secondary_account_id,
            "2026-03-23T11:00:00Z",
            Some(14.0),
            Some(20.0),
        )
        .await;
        let last_synced_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(10));
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?3,
                last_successful_sync_at = ?3
            WHERE id IN (?1, ?2)
            "#,
        )
        .bind(priority_account_id)
        .bind(secondary_account_id)
        .bind(&last_synced_at)
        .execute(&state.pool)
        .await
        .expect("seed sync times");

        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("run maintenance pass");
        timeout(Duration::from_secs(1), async {
            while requests.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("priority maintenance request should finish");
        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "overflow secondary account should not sync on the primary interval"
        );
        server.abort();
    }

    #[tokio::test]
    async fn account_actors_are_released_after_idle_commands_finish() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_api_key_account(&state.pool, "Actor cleanup").await;

        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);

        state
            .upstream_accounts
            .account_ops
            .run_update_account(
                state.clone(),
                account_id,
                UpdateUpstreamAccountRequest {
                    display_name: None,
                    group_name: None,
                    group_bound_proxy_keys: None,
                    note: Some("released".to_string()),
                    group_note: None,
                    upstream_base_url: OptionalField::Missing,
                    enabled: None,
                    is_mother: None,
                    api_key: None,
                    local_primary_limit: None,
                    local_secondary_limit: None,
                    local_limit_unit: None,
                    tag_ids: None,
                },
            )
            .await
            .expect("update account");
        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);

        state
            .upstream_accounts
            .account_ops
            .run_delete_account(state.clone(), account_id)
            .await
            .expect("delete account");
        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_keeps_missing_scope_oauth_as_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Scope OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-scope"),
            StatusCode::UNAUTHORIZED,
            "pool upstream responded with 401: Missing scopes: api.responses.write",
            None,
        )
        .await
        .expect("record route failure");

        let status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("load account status");
        assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_marks_explicit_invalidated_oauth_for_reauth() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Invalidated OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-invalidated"),
            StatusCode::FORBIDDEN,
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
            None,
        )
        .await
        .expect("record route failure");

        let status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("load account status");
        assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_keeps_bridge_exchange_oauth_as_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Bridge OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-bridge"),
            StatusCode::UNAUTHORIZED,
            "oauth bridge token exchange failed: oauth bridge responded with 502",
            None,
        )
        .await
        .expect("record route failure");

        let status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(account_id)
                .fetch_one(&pool)
                .await
                .expect("load account status");
        assert_eq!(status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_marks_402_as_hard_error_and_records_reason() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Plan Blocked Key").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-402"),
            StatusCode::PAYMENT_REQUIRED,
            "pool upstream responded with 402: subscription required",
            Some("invk_402"),
        )
        .await
        .expect("record route failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load account row")
            .expect("account should exist");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            row.last_action_source.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
        );
        assert_eq!(row.last_action_http_status, Some(402));
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
        assert!(row.cooldown_until.is_none());
    }

    #[tokio::test]
    async fn route_triggered_402_summary_and_detail_export_as_upstream_rejected() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Workspace Blocked OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-402-workspace"),
            StatusCode::PAYMENT_REQUIRED,
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            Some("invk_workspace_402"),
        )
        .await
        .expect("record route-triggered 402 failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load route-triggered 402 row")
            .expect("route-triggered 402 row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            summary.last_error.as_deref(),
            Some(
                "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}"
            )
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load route-triggered 402 detail")
            .expect("route-triggered 402 detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.failure_kind.as_deref()),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_marks_quota_429_as_hard_error_and_records_reason() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Quota Exhausted Key").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-429-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "insufficient_quota: pool upstream responded with 429: weekly cap exhausted",
            Some("invk_quota_429"),
        )
        .await
        .expect("record route failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load account row")
            .expect("account should exist");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(row.last_action_http_status, Some(429));
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert!(row.cooldown_until.is_none());
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_exports_first_plain_429_as_degraded_without_cooldown() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Degraded Plain 429").await;
        upsert_sticky_route(
            &pool,
            "sticky-degraded-first-hit",
            account_id,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("seed sticky route");

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-degraded-first-hit"),
            StatusCode::TOO_MANY_REQUESTS,
            "pool upstream responded with 429: too many requests",
            Some("invk_degraded_first_hit"),
        )
        .await
        .expect("record first degraded 429 failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load degraded row")
            .expect("degraded row exists");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            row.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE)
        );
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        );
        assert!(row.cooldown_until.is_none());
        assert_eq!(row.consecutive_route_failures, 1);
        assert!(row.temporary_route_failure_streak_started_at.is_some());
        assert_eq!(
            load_sticky_route(&pool, "sticky-degraded-first-hit")
                .await
                .expect("load sticky route after degraded hit")
                .map(|route| route.account_id),
            Some(account_id)
        );

        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    }

    #[tokio::test]
    async fn record_pool_route_http_failure_keeps_server_overloaded_as_retryable_without_cooldown()
    {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Overloaded Key").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
            Some("sticky-overloaded"),
            StatusCode::OK,
            "[upstream_response_failed] server_is_overloaded: Our servers are currently overloaded. Please try again later.",
            Some("invk_overloaded"),
        )
        .await
        .expect("record retryable overload route failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load overloaded row")
            .expect("overloaded row exists");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            row.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE)
        );
        assert_eq!(
            row.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
        );
        assert_eq!(row.last_action_http_status, Some(200));
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        );
        assert!(row.cooldown_until.is_none());
        assert_eq!(row.consecutive_route_failures, 1);
        assert!(row.temporary_route_failure_streak_started_at.is_some());

        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    }

    #[tokio::test]
    async fn record_pool_route_transport_failure_starts_temporary_cooldown_after_streak_window_expires()
     {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Cooldown Escalation").await;
        upsert_sticky_route(
            &pool,
            "sticky-degraded-cooldown",
            account_id,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("seed sticky route");

        record_pool_route_transport_failure(
            &pool,
            account_id,
            Some("sticky-degraded-cooldown"),
            "failed to contact upstream",
            Some("invk_transport_first"),
        )
        .await
        .expect("record first transport failure");

        let stale_started_at = format_utc_iso(
            Utc::now()
                - ChronoDuration::seconds(POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS + 1),
        );
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET temporary_route_failure_streak_started_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&stale_started_at)
        .execute(&pool)
        .await
        .expect("stale degraded streak start");

        record_pool_route_transport_failure(
            &pool,
            account_id,
            Some("sticky-degraded-cooldown"),
            "failed to contact upstream again",
            Some("invk_transport_second"),
        )
        .await
        .expect("record second transport failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load escalated row")
            .expect("escalated row exists");
        assert_eq!(
            row.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
        );
        assert!(row.cooldown_until.is_some());
        assert_eq!(row.consecutive_route_failures, 2);
        assert!(
            load_sticky_route(&pool, "sticky-degraded-cooldown")
                .await
                .expect("load sticky route after cooldown escalation")
                .is_none()
        );

        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    }

    #[test]
    fn classify_pool_account_http_failure_treats_usage_limit_reached_as_quota_exhausted() {
        let classification = classify_pool_account_http_failure(
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            StatusCode::TOO_MANY_REQUESTS,
            "pool upstream responded with 429: The usage limit has been reached",
        );

        assert_eq!(
            classification.disposition,
            UpstreamAccountFailureDisposition::HardUnavailable
        );
        assert_eq!(
            classification.reason_code,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
        );
        assert_eq!(
            classification.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
        );
    }

    #[tokio::test]
    async fn quota_exhausted_oauth_summary_and_detail_export_as_rate_limited() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Quota Exhausted OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-quota-exhausted"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_exhausted"),
        )
        .await
        .expect("record wrapped 429 route failure");

        let route_failure_row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load route failure row")
            .expect("route failure row exists");
        record_account_sync_recovery_blocked(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            &route_failure_row.status,
            UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
            "latest usage snapshot still shows an exhausted upstream usage limit window",
            route_failure_row.last_error.as_deref(),
            route_failure_row.last_route_failure_kind.as_deref(),
        )
        .await
        .expect("record blocked sync recovery");

        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                100.0, 300, ?3,
                64.0, 10080, ?4,
                1, 0, '0.00'
            )
            "#,
        )
        .bind(account_id)
        .bind("2026-03-24T18:00:27Z")
        .bind("2026-03-30T16:06:33Z")
        .bind("2026-04-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("insert exhausted usage sample");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        let latest = load_latest_usage_sample(&pool, account_id)
            .await
            .expect("load latest usage sample");
        let summary = build_summary_from_row(
            &row,
            latest.as_ref(),
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(
            summary.last_error.as_deref(),
            Some(
                "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached"
            )
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            summary
                .primary_window
                .as_ref()
                .map(|window| window.used_percent),
            Some(100.0)
        );
        assert_eq!(
            summary
                .primary_window
                .as_ref()
                .and_then(|window| window.resets_at.as_deref()),
            Some("2026-03-30T16:06:33Z")
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .map(|event| event.action.as_str()),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.failure_kind.as_deref()),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
    }

    #[tokio::test]
    async fn sync_triggered_402_summary_and_detail_export_as_upstream_rejected() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Workspace Sync Blocked OAuth").await;

        record_account_sync_hard_unavailable(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "upstream_http_402",
            "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}",
            PROXY_FAILURE_UPSTREAM_HTTP_402,
        )
        .await
        .expect("record sync-triggered 402 hard unavailable");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load sync-triggered 402 row")
            .expect("sync-triggered 402 row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            summary.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            summary.last_error.as_deref(),
            Some(
                "initial usage snapshot attempt with configured user agent failed: usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}"
            )
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load sync-triggered 402 detail")
            .expect("sync-triggered 402 detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .map(|event| event.action.as_str()),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some("upstream_http_402")
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.failure_kind.as_deref()),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
    }

    #[tokio::test]
    async fn stale_quota_route_failure_does_not_hide_newer_sync_error() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Stale quota marker OAuth").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-stale-quota"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_stale_quota"),
        )
        .await
        .expect("record stale wrapped 429 route failure");

        record_account_sync_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            "usage snapshot parse error after refresh",
            UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
            None,
            PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            None,
        )
        .await
        .expect("record newer sync failure");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn blocked_api_key_manual_recovery_does_not_export_as_active_rate_limited() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Manual Recovery API Key").await;

        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        record_account_sync_recovery_blocked(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset",
            Some("seed hard unavailable"),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
        )
        .await
        .expect("record blocked recovery");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load blocked api key row")
            .expect("blocked api key row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );

        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load blocked api key detail")
            .expect("blocked api key detail exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.health_status,
            UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(
            detail.summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );
    }

    #[tokio::test]
    async fn explicit_reauth_phrase_without_reauth_reason_does_not_force_needs_reauth() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "API key rejected wording").await;
        let now_iso = format_utc_iso(Utc::now());

        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_action = ?5,
                last_action_source = ?6,
                last_action_reason_code = ?7,
                last_action_reason_message = ?3,
                last_action_http_status = ?8,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind(
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
        )
        .bind(&now_iso)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind("upstream_http_403")
        .bind(403)
        .execute(&pool)
        .await
        .expect("seed non-reauth rejection state");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_ne!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_ne!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }

    #[tokio::test]
    async fn legacy_oauth_explicit_reauth_error_without_reason_code_still_exports_needs_reauth() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Legacy OAuth Reauth").await;
        let now_iso = format_utc_iso(Utc::now());

        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = NULL,
                last_action_reason_message = ?3,
                last_action_http_status = ?8,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind(
            "pool upstream responded with 403: Authentication token has been invalidated, please sign in again",
        )
        .bind(&now_iso)
        .bind(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind(403)
        .execute(&pool)
        .await
        .expect("seed legacy oauth reauth state");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }

    #[tokio::test]
    async fn current_quota_route_failure_survives_informational_account_updates() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Quota exhausted after edit").await;

        record_pool_route_http_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
            Some("sticky-quota-after-edit"),
            StatusCode::TOO_MANY_REQUESTS,
            "oauth_upstream_rejected_request: pool upstream responded with 429: The usage limit has been reached",
            Some("invk_quota_after_edit"),
        )
        .await
        .expect("record wrapped 429 route failure before edit");

        record_account_update_action(
            &pool,
            account_id,
            "account settings were updated after the quota-exhausted failure",
        )
        .await
        .expect("record account update action");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        let summary = build_summary_from_row(
            &row,
            None,
            row.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );

        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(
            summary.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED)
        );
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
    }

    async fn insert_limit_sample(
        pool: &SqlitePool,
        account_id: i64,
        captured_at: &str,
        plan_type: Option<&str>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, ?3,
                NULL, NULL, NULL,
                NULL, NULL, NULL,
                NULL, NULL, NULL
            )
            "#,
        )
        .bind(account_id)
        .bind(captured_at)
        .bind(plan_type)
        .execute(pool)
        .await
        .expect("insert limit sample");
    }

    async fn insert_limit_sample_with_usage(
        pool: &SqlitePool,
        account_id: i64,
        captured_at: &str,
        primary_used_percent: Option<f64>,
        secondary_used_percent: Option<f64>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_limit_samples (
                account_id, captured_at, limit_id, limit_name, plan_type,
                primary_used_percent, primary_window_minutes, primary_resets_at,
                secondary_used_percent, secondary_window_minutes, secondary_resets_at,
                credits_has_credits, credits_unlimited, credits_balance
            ) VALUES (
                ?1, ?2, NULL, NULL, 'team',
                ?3, 300, NULL,
                ?4, 10080, NULL,
                NULL, NULL, NULL
            )
            "#,
        )
        .bind(account_id)
        .bind(captured_at)
        .bind(primary_used_percent)
        .bind(secondary_used_percent)
        .execute(pool)
        .await
        .expect("insert limit sample with usage");
    }

    async fn seed_route_cooldown(
        pool: &SqlitePool,
        account_id: i64,
        failure_kind: &str,
        cooldown_secs: i64,
    ) {
        let now = Utc::now();
        let now_iso = format_utc_iso(now);
        let cooldown_until = format_utc_iso(now + ChronoDuration::seconds(cooldown_secs));
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("seed route cooldown")
        .bind(&now_iso)
        .bind(failure_kind)
        .bind(&cooldown_until)
        .execute(pool)
        .await
        .expect("seed route cooldown");
    }

    async fn seed_hard_unavailable_route_failure(
        pool: &SqlitePool,
        account_id: i64,
        status: &str,
        failure_kind: &str,
        reason_code: &str,
        http_status: Option<i64>,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = NULL,
                consecutive_route_failures = 1,
                temporary_route_failure_streak_started_at = NULL,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = ?8,
                last_action_reason_message = ?3,
                last_action_http_status = ?9,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(status)
        .bind("seed hard unavailable")
        .bind(&now_iso)
        .bind(failure_kind)
        .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
        .bind(reason_code)
        .bind(http_status)
        .execute(pool)
        .await
        .expect("seed hard unavailable");
    }

    #[tokio::test]
    async fn record_pool_route_success_does_not_clear_newer_route_failure_state() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Stale Success Guard").await;
        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        record_pool_route_success(
            &pool,
            account_id,
            Utc::now() - ChronoDuration::minutes(5),
            Some("sticky-stale-success"),
            Some("invk_stale_success"),
        )
        .await
        .expect("record stale route success");

        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after stale success")
            .expect("row exists after stale success");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE)
        );
        assert!(
            load_sticky_route(&pool, "sticky-stale-success")
                .await
                .expect("load sticky route after stale success")
                .is_none()
        );
    }

    #[tokio::test]
    async fn mark_account_sync_success_preserves_route_cooldown_state() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Cooldown OAuth").await;
        seed_route_cooldown(
            &pool,
            account_id,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            300,
        )
        .await;

        let before = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row before sync")
            .expect("row exists before sync");
        mark_account_sync_success(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
            SyncSuccessRouteState::PreserveFailureState,
        )
        .await
        .expect("mark sync success");
        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after sync")
            .expect("row exists after sync");

        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_synced_at.is_some());
        assert!(after.last_successful_sync_at.is_some());
        assert_eq!(after.last_route_failure_at, before.last_route_failure_at);
        assert_eq!(
            after.last_route_failure_kind,
            before.last_route_failure_kind
        );
        assert_eq!(after.cooldown_until, before.cooldown_until);
        assert_eq!(
            after.consecutive_route_failures,
            before.consecutive_route_failures
        );
    }

    #[tokio::test]
    async fn mark_account_sync_success_clears_hard_unavailable_state_when_requested() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Recovered OAuth").await;
        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        mark_account_sync_success(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
            SyncSuccessRouteState::ClearFailureState,
        )
        .await
        .expect("mark sync success");

        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after sync success")
            .expect("row exists after sync success");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_error.is_none());
        assert!(after.last_route_failure_kind.is_none());
        assert!(after.cooldown_until.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
        );
    }

    #[tokio::test]
    async fn sync_api_key_account_preserves_route_cooldown_state() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Cooldown API Key").await;
        seed_route_cooldown(
            &pool,
            account_id,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            300,
        )
        .await;
        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load api key row")
            .expect("api key row exists");

        sync_api_key_account(&pool, &row, SyncCause::Manual)
            .await
            .expect("sync api key account");
        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after api key sync")
            .expect("row exists after api key sync");

        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        );
        assert!(after.cooldown_until.is_some());
        assert_eq!(after.consecutive_route_failures, 1);
    }

    #[tokio::test]
    async fn sync_api_key_account_keeps_hard_unavailable_accounts_blocked() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Blocked API Key").await;
        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load api key row")
            .expect("api key row exists");

        sync_api_key_account(&pool, &row, SyncCause::Manual)
            .await
            .expect("sync api key account");
        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after api key sync")
            .expect("row exists after api key sync");

        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert!(after.last_synced_at.is_some());
        assert!(after.last_successful_sync_at.is_none());
        assert_eq!(after.last_error.as_deref(), Some("seed hard unavailable"));
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
    }

    #[tokio::test]
    async fn sync_api_key_account_clears_stale_manual_recovery_marker_on_active_rows() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Active API Key With Stale Marker").await;
        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        mark_account_sync_success(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            SyncSuccessRouteState::PreserveFailureState,
        )
        .await
        .expect("mark legacy sync success");
        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load api key row")
            .expect("api key row exists");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );

        sync_api_key_account(&pool, &row, SyncCause::Maintenance)
            .await
            .expect("sync api key account");
        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after api key sync")
            .expect("row exists after api key sync");

        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_route_failure_kind.is_none());
        assert!(after.cooldown_until.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
        );
    }

    #[tokio::test]
    async fn updating_api_key_reactivates_manually_recoverable_account() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_api_key_account(&state.pool, "Recoverable API Key").await;
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        state
            .upstream_accounts
            .account_ops
            .run_update_account(
                state.clone(),
                account_id,
                UpdateUpstreamAccountRequest {
                    display_name: None,
                    group_name: None,
                    group_bound_proxy_keys: None,
                    note: None,
                    group_note: None,
                    upstream_base_url: OptionalField::Missing,
                    enabled: None,
                    is_mother: None,
                    api_key: Some("sk-live-new".to_string()),
                    local_primary_limit: None,
                    local_secondary_limit: None,
                    local_limit_unit: None,
                    tag_ids: None,
                },
            )
            .await
            .expect("update api key account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load row after api key update")
            .expect("row exists after api key update");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_error.is_none());
        assert!(after.last_route_failure_kind.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED)
        );
    }

    #[tokio::test]
    async fn oauth_sync_keeps_quota_exhausted_accounts_blocked_until_snapshot_recovers() {
        let (base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 100,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Exhausted OAuth",
            "exhausted@example.com",
            "org_exhausted",
            "user_exhausted",
        )
        .await;
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        let row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");

        sync_oauth_account(&state, &row, SyncCause::Manual)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after sync")
            .expect("oauth row exists after sync");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert!(after.last_synced_at.is_some());
        assert!(after.last_successful_sync_at.is_none());
        assert_eq!(after.last_error.as_deref(), Some("seed hard unavailable"));
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_ignores_stale_input_row_after_newer_quota_hard_stop() {
        let (base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 100,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Stale OAuth Input Row",
            "stale-input@example.com",
            "org_stale_input",
            "user_stale_input",
        )
        .await;
        let stale_row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after stale sync")
            .expect("oauth row exists after stale sync");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_demotes_active_stale_quota_marker_when_snapshot_is_still_exhausted() {
        let (base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 100,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Legacy Active Exhausted OAuth",
            "legacy-exhausted@example.com",
            "org_legacy_exhausted",
            "user_legacy_exhausted",
        )
        .await;
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        mark_account_sync_success(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            SyncSuccessRouteState::PreserveFailureState,
        )
        .await
        .expect("mark legacy sync success");
        let row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");
        assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            row.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );

        sync_oauth_account(&state, &row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after sync")
            .expect("oauth row exists after sync");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_reactivates_quota_exhausted_account_once_snapshot_recovers() {
        let (base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 42,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Recovered OAuth Sync",
            "recovered@example.com",
            "org_recovered",
            "user_recovered",
        )
        .await;
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        let row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");

        sync_oauth_account(&state, &row, SyncCause::Manual)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after recovery")
            .expect("oauth row exists after recovery");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_error.is_none());
        assert!(after.last_route_failure_kind.is_none());
        assert!(after.last_successful_sync_at.is_some());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
        );
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing() {
        let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::FORBIDDEN,
                        json!({
                            "error": {
                                "message": "Authentication token has been invalidated, please sign in again"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-access-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "reauth-required@example.com",
                        Some("org_retry_reauth"),
                        Some("user_retry_reauth"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
        let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Retry Needs Reauth OAuth",
            "reauth-required@example.com",
            "org_retry_reauth",
            "user_retry_reauth",
        )
        .await;
        let row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");

        sync_oauth_account(&state, &row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after retry failure")
            .expect("oauth row exists after retry failure");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert!(after.last_synced_at.is_some());
        assert!(after.last_successful_sync_at.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
        );
        assert_eq!(after.last_action_http_status, Some(403));
        assert_eq!(
            after.last_error.as_deref(),
            Some(
                "usage endpoint returned 403 Forbidden: Authentication token has been invalidated, please sign in again"
            )
        );
        assert!(after.last_action_at.is_some());

        let decrypted = decrypt_credentials(
            crypto_key,
            after
                .encrypted_credentials
                .as_deref()
                .expect("encrypted oauth credentials"),
        )
        .expect("decrypt refreshed credentials");
        let StoredCredentials::Oauth(credentials) = decrypted else {
            panic!("unexpected credential kind after refresh")
        };
        assert_eq!(credentials.access_token, "refreshed-access-token");
        assert_eq!(credentials.refresh_token, "refresh-token-rotated");

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        let detail = load_upstream_account_detail(&state.pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
        );
        assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
        assert_eq!(
            detail
                .recent_actions
                .first()
                .map(|event| event.action.as_str()),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.reason_code.as_deref()),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
        );
        assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing()
     {
        let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::BAD_GATEWAY,
                        json!({
                            "error": {
                                "message": "gateway temporarily unavailable"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-temporary-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "transport-failure@example.com",
                        Some("org_retry_gateway"),
                        Some("user_retry_gateway"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
        let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Retry Gateway Failure OAuth",
            "transport-failure@example.com",
            "org_retry_gateway",
            "user_retry_gateway",
        )
        .await;
        seed_route_cooldown(
            &state.pool,
            account_id,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            300,
        )
        .await;
        let row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");

        sync_oauth_account(&state, &row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after gateway failure")
            .expect("oauth row exists after gateway failure");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_synced_at.is_some());
        assert!(after.last_successful_sync_at.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some("upstream_http_5xx")
        );
        assert_eq!(after.last_action_http_status, Some(502));
        assert_eq!(
            after.last_error.as_deref(),
            Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
        );
        assert!(after.last_action_at.is_some());
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        );
        assert!(after.cooldown_until.is_some());
        assert_eq!(after.consecutive_route_failures, 1);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        let detail = load_upstream_account_detail(&state.pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
        );
        assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
        assert_eq!(
            detail
                .recent_actions
                .first()
                .map(|event| event.action.as_str()),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            detail
                .recent_actions
                .first()
                .and_then(|event| event.http_status),
            Some(502)
        );
        assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_retry_after_refresh_preserves_quota_marker_from_current_db_state() {
        let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::BAD_GATEWAY,
                        json!({
                            "error": {
                                "message": "gateway temporarily unavailable"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-quota-preserving-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "retry-quota-preserve@example.com",
                        Some("org_retry_quota_preserve"),
                        Some("user_retry_quota_preserve"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
        let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Retry Gateway Quota Preserve OAuth",
            "retry-quota-preserve@example.com",
            "org_retry_quota_preserve",
            "user_retry_quota_preserve",
        )
        .await;
        let stale_row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after gateway failure")
            .expect("oauth row exists after gateway failure");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert!(after.last_synced_at.is_some());
        assert!(after.last_successful_sync_at.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some("upstream_http_5xx")
        );
        assert_eq!(after.last_action_http_status, Some(502));
        assert_eq!(
            after.last_error.as_deref(),
            Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
        );
        assert!(after.last_action_at.is_some());
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(after.last_route_failure_at, after.last_error_at);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        let detail = load_upstream_account_detail(&state.pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_refresh_failure_preserves_quota_marker_from_current_db_state() {
        let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![(
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                )],
                json!({
                    "unexpected": "shape"
                }),
            )
            .await;
        let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Refresh Failure Quota Preserve OAuth",
            "refresh-quota-preserve@example.com",
            "org_refresh_quota_preserve",
            "user_refresh_quota_preserve",
        )
        .await;
        let stale_row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after refresh failure")
            .expect("oauth row exists after refresh failure");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
        );
        assert_eq!(after.last_action_http_status, None);
        assert!(
            after
                .last_error
                .as_deref()
                .is_some_and(|message| message.contains("failed to decode OAuth token response"))
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(after.last_route_failure_at, after.last_error_at);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        let detail = load_upstream_account_detail(&state.pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn oauth_sync_direct_fetch_failure_preserves_quota_marker_from_current_db_state() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
            StatusCode::BAD_GATEWAY,
            json!({
                "error": {
                    "message": "gateway temporarily unavailable"
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Direct Failure Quota Preserve OAuth",
            "direct-quota-preserve@example.com",
            "org_direct_quota_preserve",
            "user_direct_quota_preserve",
        )
        .await;
        let stale_row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;

        sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after direct fetch failure")
            .expect("oauth row exists after direct fetch failure");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some("upstream_http_5xx")
        );
        assert_eq!(after.last_action_http_status, Some(502));
        assert!(
            after
                .last_error
                .as_deref()
                .is_some_and(|message| message.contains("502 Bad Gateway"))
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(after.last_route_failure_at, after.last_error_at);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

        let detail = load_upstream_account_detail(&state.pool, account_id)
            .await
            .expect("load detail export")
            .expect("detail export exists");
        assert_eq!(
            detail.summary.display_status,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        );
        assert_eq!(
            detail.summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
        server.abort();
    }

    #[tokio::test]
    async fn classified_sync_failure_preserves_existing_route_cooldown_across_new_error_timestamp()
    {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Preserved Cooldown OAuth").await;
        let previous_failure_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(2));
        let cooldown_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(5));

        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                cooldown_until = ?6,
                consecutive_route_failures = 1,
                last_action = ?7,
                last_action_source = ?8,
                last_action_reason_code = ?9,
                last_action_reason_message = ?3,
                last_action_http_status = ?10,
                last_action_at = ?4,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("seed preserved cooldown")
        .bind(&previous_failure_at)
        .bind(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        .bind(&cooldown_until)
        .bind(UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL)
        .bind(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
        .bind(429)
        .execute(&pool)
        .await
        .expect("seed preserved cooldown row");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load seeded cooldown row")
            .expect("seeded cooldown row exists");
        record_classified_account_sync_failure(
            &pool,
            &row,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
        )
        .await
        .expect("record classified retry failure");

        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load cooldown row after retry failure")
            .expect("cooldown row after retry failure exists");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some("upstream_http_5xx")
        );
        assert_eq!(after.last_action_http_status, Some(502));
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        );
        assert_eq!(after.last_route_failure_at, after.last_error_at);
        assert_ne!(
            after.last_route_failure_at.as_deref(),
            Some(previous_failure_at.as_str())
        );

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }

    #[tokio::test]
    async fn classified_sync_failure_preserves_quota_marker_from_current_syncing_row() {
        let pool = test_pool().await;
        let account_id = insert_oauth_account(&pool, "Quota Syncing Preserve").await;

        seed_hard_unavailable_route_failure(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        set_account_status(&pool, account_id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None)
            .await
            .expect("mark row syncing");

        let current_row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load current syncing row")
            .expect("current syncing row exists");
        assert_eq!(current_row.status, UPSTREAM_ACCOUNT_STATUS_SYNCING);

        record_classified_account_sync_failure(
            &pool,
            &current_row,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            "usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable",
        )
        .await
        .expect("record retry failure against syncing row");

        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load syncing row after retry failure")
            .expect("syncing row after retry failure exists");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some("upstream_http_5xx")
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        );
        assert_eq!(after.last_route_failure_at, after.last_error_at);

        let summary = build_summary_from_row(
            &after,
            None,
            after.last_activity_at.clone(),
            vec![],
            None,
            0,
            Utc::now(),
        );
        assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
        assert_eq!(
            summary.work_status,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
        );
        assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    }

    #[tokio::test]
    async fn oauth_sync_proactively_quarantines_snapshot_exhausted_account_without_prior_route_failure()
     {
        let (base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 100,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Sync Snapshot Exhausted",
            "snapshot-exhausted@example.com",
            "org_snapshot_exhausted",
            "user_snapshot_exhausted",
        )
        .await;
        let row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row")
            .expect("oauth row exists");

        sync_oauth_account(&state, &row, SyncCause::Maintenance)
            .await
            .expect("sync oauth account");

        let after = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth row after proactive quarantine")
            .expect("oauth row exists after proactive quarantine");
        assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
        assert!(after.last_successful_sync_at.is_none());
        assert_eq!(
            after.last_action.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        );
        assert_eq!(
            after.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
        );
        assert_eq!(
            after.last_route_failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED)
        );
        server.abort();
    }

    #[tokio::test]
    async fn resolver_short_circuits_when_only_persisted_snapshot_exhausted_accounts_remain() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let first = insert_api_key_account(&state.pool, "Exhausted A").await;
        let second = insert_api_key_account(&state.pool, "Exhausted B").await;
        let third = insert_api_key_account(&state.pool, "Exhausted C").await;
        let now_iso = format_utc_iso(Utc::now());
        for account_id in [first, second, third] {
            insert_limit_sample_with_usage(
                &state.pool,
                account_id,
                &now_iso,
                Some(100.0),
                Some(40.0),
            )
            .await;
        }

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        assert!(matches!(resolution, PoolAccountResolution::RateLimited));
    }

    #[tokio::test]
    async fn resolver_skips_persisted_snapshot_exhausted_account_before_routing() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let exhausted = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Exhausted Candidate",
            "exhausted-candidate@example.com",
            "org_exhausted_candidate",
            "user_exhausted_candidate",
        )
        .await;
        let available = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Available Candidate",
            "available-candidate@example.com",
            "org_available_candidate",
            "user_available_candidate",
        )
        .await;
        let now_iso = format_utc_iso(Utc::now());
        insert_limit_sample_with_usage(&state.pool, exhausted, &now_iso, Some(100.0), Some(20.0))
            .await;
        insert_limit_sample_with_usage(&state.pool, available, &now_iso, Some(42.0), Some(10.0))
            .await;

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        let PoolAccountResolution::Resolved(account) = resolution else {
            panic!("expected resolver to pick an available account");
        };
        assert_eq!(account.account_id, available);
    }

    #[tokio::test]
    async fn resolver_keeps_quota_exhausted_accounts_in_rate_limited_terminal_state_after_sync_block()
     {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_api_key_account(&state.pool, "Quota Exhausted Resolver").await;
        seed_hard_unavailable_route_failure(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            Some(429),
        )
        .await;
        record_account_sync_recovery_blocked(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset",
            Some("seed hard unavailable"),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
        )
        .await
        .expect("record blocked recovery");

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        assert!(matches!(resolution, PoolAccountResolution::RateLimited));
    }

    #[tokio::test]
    async fn resolver_skips_candidate_when_group_has_no_bound_proxy_keys() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let blocked = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Blocked Missing Binding",
            "blocked-missing-binding@example.com",
            "org_blocked_missing_binding",
            "user_blocked_missing_binding",
        )
        .await;
        let healthy = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Healthy Candidate",
            "healthy-candidate@example.com",
            "org_healthy_candidate",
            "user_healthy_candidate",
        )
        .await;
        set_test_account_group_name(&state.pool, blocked, Some("missing-bindings")).await;
        let now_iso = format_utc_iso(Utc::now());
        insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
        insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(80.0), Some(10.0))
            .await;

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        let PoolAccountResolution::Resolved(account) = resolution else {
            panic!("expected resolver to skip missing-binding group and pick healthy account");
        };
        assert_eq!(account.account_id, healthy);
    }

    #[tokio::test]
    async fn resolver_skips_candidate_when_group_has_only_unselectable_bound_proxies() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let blocked = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Blocked Unselectable Binding",
            "blocked-unselectable-binding@example.com",
            "org_blocked_unselectable_binding",
            "user_blocked_unselectable_binding",
        )
        .await;
        let healthy = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Healthy Fallback",
            "healthy-fallback@example.com",
            "org_healthy_fallback",
            "user_healthy_fallback",
        )
        .await;
        set_test_account_group_name(&state.pool, blocked, Some("staging")).await;
        upsert_test_group_binding(
            &state.pool,
            "staging",
            vec!["unselectable-bound-node".to_string()],
        )
        .await;
        let now_iso = format_utc_iso(Utc::now());
        insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;
        insert_limit_sample_with_usage(&state.pool, healthy, &now_iso, Some(70.0), Some(10.0))
            .await;

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        let PoolAccountResolution::Resolved(account) = resolution else {
            panic!("expected resolver to skip unselectable group and pick healthy account");
        };
        assert_eq!(account.account_id, healthy);
    }

    #[tokio::test]
    async fn resolver_returns_specific_group_proxy_error_when_only_bad_groups_remain() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Only Missing Binding",
            "only-missing-binding@example.com",
            "org_only_missing_binding",
            "user_only_missing_binding",
        )
        .await;
        set_test_account_group_name(&state.pool, account, Some("missing-bindings")).await;

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
            panic!("expected specific group proxy error");
        };
        assert_eq!(
            message,
            "upstream account group \"missing-bindings\" has no bound forward proxy nodes; bind at least one proxy node to the group"
        );
    }

    #[tokio::test]
    async fn resolver_prefers_group_proxy_error_over_rate_limited_pool_when_no_healthy_candidates_remain()
     {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let rate_limited = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Rate Limited Candidate",
            "rate-limited-candidate@example.com",
            "org_rate_limited_candidate",
            "user_rate_limited_candidate",
        )
        .await;
        let blocked = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Blocked Missing Binding",
            "blocked-missing-binding-mixed@example.com",
            "org_blocked_missing_binding_mixed",
            "user_blocked_missing_binding_mixed",
        )
        .await;
        set_test_account_group_name(&state.pool, blocked, Some("missing-bindings")).await;
        let now_iso = format_utc_iso(Utc::now());
        insert_limit_sample_with_usage(
            &state.pool,
            rate_limited,
            &now_iso,
            Some(100.0),
            Some(50.0),
        )
        .await;
        insert_limit_sample_with_usage(&state.pool, blocked, &now_iso, Some(1.0), Some(1.0)).await;

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
            panic!("expected group proxy error to win over mixed rate-limited pool");
        };
        assert_eq!(
            message,
            "upstream account group \"missing-bindings\" has no bound forward proxy nodes; bind at least one proxy node to the group"
        );
    }

    #[tokio::test]
    async fn resolver_summarizes_multiple_group_proxy_errors_when_only_bad_groups_remain() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let missing_binding = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Missing Binding Group",
            "missing-binding-group@example.com",
            "org_missing_binding_group",
            "user_missing_binding_group",
        )
        .await;
        let unselectable = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Unselectable Binding Group",
            "unselectable-binding-group@example.com",
            "org_unselectable_binding_group",
            "user_unselectable_binding_group",
        )
        .await;
        set_test_account_group_name(&state.pool, missing_binding, Some("group-a")).await;
        set_test_account_group_name(&state.pool, unselectable, Some("group-b")).await;
        upsert_test_group_binding(
            &state.pool,
            "group-b",
            vec!["unselectable-bound-node".to_string()],
        )
        .await;
        let now_iso = format_utc_iso(Utc::now());
        insert_limit_sample_with_usage(
            &state.pool,
            missing_binding,
            &now_iso,
            Some(1.0),
            Some(1.0),
        )
        .await;
        insert_limit_sample_with_usage(&state.pool, unselectable, &now_iso, Some(5.0), Some(1.0))
            .await;

        let resolution = resolve_pool_account_for_request(&state, None, &[], &HashSet::new())
            .await
            .expect("resolve pool account");
        let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
            panic!("expected summarized group proxy error");
        };
        assert!(message.contains(
            "upstream account group \"group-a\" has no bound forward proxy nodes; bind at least one proxy node to the group"
        ));
        assert!(
            message.contains(
                "plus 1 additional upstream account group routing configuration issue(s)"
            )
        );
    }

    #[tokio::test]
    async fn resolver_can_cut_out_from_group_proxy_blocked_sticky_account_when_allowed() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let sticky_account = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Sticky Invalid Group",
            "sticky-invalid-group@example.com",
            "org_sticky_invalid_group",
            "user_sticky_invalid_group",
        )
        .await;
        let fallback_account = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Fallback Healthy Group",
            "fallback-healthy-group@example.com",
            "org_fallback_healthy_group",
            "user_fallback_healthy_group",
        )
        .await;
        set_test_account_group_name(&state.pool, sticky_account, Some("sticky-missing")).await;
        upsert_sticky_route(
            &state.pool,
            "sticky-group-proxy-blocked",
            sticky_account,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("upsert sticky route");

        let resolution = resolve_pool_account_for_request(
            &state,
            Some("sticky-group-proxy-blocked"),
            &[],
            &HashSet::new(),
        )
        .await
        .expect("resolve pool account");
        let PoolAccountResolution::Resolved(account) = resolution else {
            panic!("expected resolver to cut out from blocked sticky account");
        };
        assert_eq!(account.account_id, fallback_account);
    }

    #[tokio::test]
    async fn resolver_returns_group_proxy_error_for_sticky_account_when_cut_out_is_forbidden() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let sticky_account = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Sticky Invalid Locked Group",
            "sticky-invalid-locked-group@example.com",
            "org_sticky_invalid_locked_group",
            "user_sticky_invalid_locked_group",
        )
        .await;
        let _fallback_account = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Ignored Healthy Group",
            "ignored-healthy-group@example.com",
            "org_ignored_healthy_group",
            "user_ignored_healthy_group",
        )
        .await;
        set_test_account_group_name(&state.pool, sticky_account, Some("sticky-missing")).await;
        let lock_tag = insert_tag(
            &state.pool,
            "sticky-lock",
            &TagRoutingRule {
                guard_enabled: false,
                lookback_hours: None,
                max_conversations: None,
                allow_cut_out: false,
                allow_cut_in: true,
            },
        )
        .await
        .expect("insert lock tag");
        sync_account_tag_links(&state.pool, sticky_account, &[lock_tag.summary.id])
            .await
            .expect("attach lock tag");
        upsert_sticky_route(
            &state.pool,
            "sticky-group-proxy-locked",
            sticky_account,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("upsert sticky route");

        let resolution = resolve_pool_account_for_request(
            &state,
            Some("sticky-group-proxy-locked"),
            &[],
            &HashSet::new(),
        )
        .await
        .expect("resolve pool account");
        let PoolAccountResolution::BlockedByPolicy(message) = resolution else {
            panic!("expected sticky group proxy error when cut-out is forbidden");
        };
        assert_eq!(
            message,
            "upstream account group \"sticky-missing\" has no bound forward proxy nodes; bind at least one proxy node to the group"
        );
    }

    #[tokio::test]
    async fn resolver_prefers_sticky_cut_in_policy_over_group_proxy_error() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let sticky_source = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Sticky Source",
            "sticky-source@example.com",
            "org_sticky_source",
            "user_sticky_source",
        )
        .await;
        let blocked_target = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Sticky Cut In Blocked",
            "sticky-cut-in-blocked@example.com",
            "org_sticky_cut_in_blocked",
            "user_sticky_cut_in_blocked",
        )
        .await;
        set_test_account_group_name(&state.pool, blocked_target, Some("sticky-cut-in-missing"))
            .await;
        let no_cut_in_tag = insert_tag(
            &state.pool,
            "sticky-no-cut-in",
            &TagRoutingRule {
                guard_enabled: false,
                lookback_hours: None,
                max_conversations: None,
                allow_cut_out: true,
                allow_cut_in: false,
            },
        )
        .await
        .expect("insert no cut-in tag");
        sync_account_tag_links(&state.pool, blocked_target, &[no_cut_in_tag.summary.id])
            .await
            .expect("attach no cut-in tag");
        upsert_sticky_route(
            &state.pool,
            "sticky-cut-in-policy-first",
            sticky_source,
            &format_utc_iso(Utc::now()),
        )
        .await
        .expect("upsert sticky route");

        let resolution = resolve_pool_account_for_request(
            &state,
            Some("sticky-cut-in-policy-first"),
            &[sticky_source],
            &HashSet::new(),
        )
        .await
        .expect("resolve pool account");
        assert!(matches!(resolution, PoolAccountResolution::Unavailable));
    }

    #[tokio::test]
    async fn update_oauth_login_session_preserves_pending_url_and_persists_metadata() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let tag_id = insert_tag(&state.pool, "pending-sync", &test_tag_routing_rule())
            .await
            .expect("insert tag")
            .summary
            .id;
        insert_test_oauth_mailbox_session(
            &state.pool,
            "mailbox-session-1",
            "pending-sync@mail-tw.707079.xyz",
            OAUTH_MAILBOX_SOURCE_ATTACHED,
        )
        .await;

        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Original Pending".to_string()),
                group_name: Some("alpha".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before".to_string()),
                group_note: Some("alpha note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let updated =
            update_oauth_login_session(
                State(state.clone()),
                HeaderMap::new(),
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Updated Pending".to_string()),
                    group_name: OptionalField::Value("beta".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("after".to_string()),
                    group_note: OptionalField::Value("beta shared".to_string()),
                    tag_ids: OptionalField::Value(vec![tag_id]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Value("mailbox-session-1".to_string()),
                    mailbox_address: OptionalField::Value(
                        "pending-sync@mail-tw.707079.xyz".to_string(),
                    ),
                }),
            )
            .await
            .expect("update oauth login session")
            .0;

        assert_eq!(updated.login_id, created.login_id);
        assert_eq!(updated.auth_url, created.auth_url);
        assert_eq!(updated.redirect_uri, created.redirect_uri);
        assert_eq!(updated.expires_at, created.expires_at);

        let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
            .await
            .expect("load stored login session")
            .expect("stored login session should exist");
        assert_eq!(stored.display_name.as_deref(), Some("Updated Pending"));
        assert_eq!(stored.group_name.as_deref(), Some("beta"));
        assert_eq!(stored.note.as_deref(), Some("after"));
        assert_eq!(stored.group_note.as_deref(), Some("beta shared"));
        assert_eq!(stored.is_mother, 1);
        assert_eq!(
            parse_tag_ids_json(stored.tag_ids_json.as_deref()),
            vec![tag_id]
        );
        assert_eq!(
            stored.mailbox_session_id.as_deref(),
            Some("mailbox-session-1")
        );
        assert_eq!(
            stored.mailbox_address.as_deref(),
            Some("pending-sync@mail-tw.707079.xyz")
        );
    }

    #[tokio::test]
    async fn update_oauth_login_session_ignores_stale_baseline_updates() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Ordered Pending".to_string()),
                group_name: Some("alpha".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before".to_string()),
                group_note: Some("alpha note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let mut newer_headers = HeaderMap::new();
        newer_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let newer =
            update_oauth_login_session(
                State(state.clone()),
                newer_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Newest Pending".to_string()),
                    group_name: OptionalField::Value("beta".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("newest note".to_string()),
                    group_note: OptionalField::Value("beta note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect("apply newer oauth login session update")
            .0;
        assert_ne!(newer.updated_at, created.updated_at);
        let newer_updated_at = newer.updated_at.clone();

        let mut stale_headers = HeaderMap::new();
        stale_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let stale =
            update_oauth_login_session(
                State(state.clone()),
                stale_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Stale Pending".to_string()),
                    group_name: OptionalField::Value("gamma".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("stale note".to_string()),
                    group_note: OptionalField::Value("gamma note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(false),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect("stale oauth login session update should be ignored")
            .0;

        assert_eq!(stale.login_id, created.login_id);
        assert_eq!(stale.updated_at, newer_updated_at);

        let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load stored login session")
            .expect("stored login session should exist");
        assert_eq!(stored.display_name.as_deref(), Some("Newest Pending"));
        assert_eq!(stored.group_name.as_deref(), Some("beta"));
        assert_eq!(stored.note.as_deref(), Some("newest note"));
        assert_eq!(stored.group_note.as_deref(), Some("beta note"));
        assert_eq!(stored.is_mother, 1);
        assert_eq!(stored.updated_at, newer_updated_at);
    }

    #[tokio::test]
    async fn update_oauth_login_session_preserves_omitted_fields() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let tag_id = insert_tag(&state.pool, "partial-sync", &test_tag_routing_rule())
            .await
            .expect("insert tag")
            .summary
            .id;
        insert_test_oauth_mailbox_session(
            &state.pool,
            "mailbox-session-partial",
            "partial-sync@mail-tw.707079.xyz",
            OAUTH_MAILBOX_SOURCE_ATTACHED,
        )
        .await;

        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Keep Me".to_string()),
                group_name: Some("partial-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before partial patch".to_string()),
                group_note: Some("partial draft note".to_string()),
                account_id: None,
                tag_ids: vec![tag_id],
                is_mother: Some(true),
                mailbox_session_id: Some("mailbox-session-partial".to_string()),
                mailbox_address: Some("partial-sync@mail-tw.707079.xyz".to_string()),
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let updated = update_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(created.login_id.clone()),
            Json(UpdateOauthLoginSessionRequest {
                display_name: OptionalField::Missing,
                group_name: OptionalField::Missing,
                group_bound_proxy_keys: OptionalField::Missing,
                note: OptionalField::Value("after partial patch".to_string()),
                group_note: OptionalField::Missing,
                tag_ids: OptionalField::Missing,
                is_mother: OptionalField::Missing,
                mailbox_session_id: OptionalField::Missing,
                mailbox_address: OptionalField::Missing,
            }),
        )
        .await
        .expect("update oauth login session")
        .0;

        assert_eq!(updated.login_id, created.login_id);
        assert_eq!(updated.auth_url, created.auth_url);
        assert_eq!(updated.redirect_uri, created.redirect_uri);
        assert_eq!(updated.expires_at, created.expires_at);

        let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
            .await
            .expect("load stored login session")
            .expect("stored login session should exist");
        assert_eq!(stored.display_name.as_deref(), Some("Keep Me"));
        assert_eq!(stored.group_name.as_deref(), Some("partial-group"));
        assert_eq!(stored.note.as_deref(), Some("after partial patch"));
        assert_eq!(stored.group_note.as_deref(), Some("partial draft note"));
        assert_eq!(stored.is_mother, 1);
        assert_eq!(
            parse_tag_ids_json(stored.tag_ids_json.as_deref()),
            vec![tag_id]
        );
        assert_eq!(
            stored.mailbox_session_id.as_deref(),
            Some("mailbox-session-partial")
        );
        assert_eq!(
            stored.mailbox_address.as_deref(),
            Some("partial-sync@mail-tw.707079.xyz")
        );
    }

    #[tokio::test]
    async fn update_oauth_login_session_clears_omitted_group_note_when_group_changes() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Move Draft Group".to_string()),
                group_name: Some("before-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before note".to_string()),
                group_note: Some("before draft note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let updated =
            update_oauth_login_session(
                State(state.clone()),
                HeaderMap::new(),
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Missing,
                    group_name: OptionalField::Value("after-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Missing,
                    group_note: OptionalField::Missing,
                    tag_ids: OptionalField::Missing,
                    is_mother: OptionalField::Missing,
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect("update oauth login session")
            .0;

        assert_eq!(updated.login_id, created.login_id);
        assert_eq!(updated.auth_url, created.auth_url);
        assert_eq!(updated.redirect_uri, created.redirect_uri);
        assert_eq!(updated.expires_at, created.expires_at);

        let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
            .await
            .expect("load stored login session")
            .expect("stored login session should exist");
        assert_eq!(stored.group_name.as_deref(), Some("after-group"));
        assert_eq!(stored.group_note, None);
        assert_eq!(stored.note.as_deref(), Some("before note"));
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_group_removal() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Clear Group Note".to_string()),
                group_name: Some("draft-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before clearing group".to_string()),
                group_note: Some("draft group note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let updated = update_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(created.login_id.clone()),
            Json(UpdateOauthLoginSessionRequest {
                display_name: OptionalField::Missing,
                group_name: OptionalField::Value(String::new()),
                group_bound_proxy_keys: OptionalField::Value(vec![]),
                note: OptionalField::Missing,
                group_note: OptionalField::Missing,
                tag_ids: OptionalField::Missing,
                is_mother: OptionalField::Missing,
                mailbox_session_id: OptionalField::Missing,
                mailbox_address: OptionalField::Missing,
            }),
        )
        .await
        .expect_err("group removal should be rejected");
        assert_eq!(updated.0, StatusCode::BAD_REQUEST);
        assert_eq!(updated.1, "groupName is required for upstream accounts");

        let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load stored login session")
            .expect("stored login session should exist");
        assert_eq!(stored.group_name.as_deref(), Some("draft-group"));
        assert_eq!(stored.group_note.as_deref(), Some("draft group note"));
        assert_eq!(stored.note.as_deref(), Some("before clearing group"));
    }

    #[tokio::test]
    async fn updated_oauth_login_session_metadata_is_used_when_callback_persists_account() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let tag_id = insert_tag(&state.pool, "callback-sync", &test_tag_routing_rule())
            .await
            .expect("insert tag")
            .summary
            .id;
        insert_test_oauth_mailbox_session(
            &state.pool,
            "mailbox-session-2",
            "callback-sync@mail-tw.707079.xyz",
            OAUTH_MAILBOX_SOURCE_ATTACHED,
        )
        .await;

        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Before Patch".to_string()),
                group_name: Some("old-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before note".to_string()),
                group_note: Some("old group note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let _ =
            update_oauth_login_session(
                State(state.clone()),
                HeaderMap::new(),
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("After Patch".to_string()),
                    group_name: OptionalField::Value("new-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("after note".to_string()),
                    group_note: OptionalField::Value("draft group note".to_string()),
                    tag_ids: OptionalField::Value(vec![tag_id]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Value("mailbox-session-2".to_string()),
                    mailbox_address: OptionalField::Value(
                        "callback-sync@mail-tw.707079.xyz".to_string(),
                    ),
                }),
            )
            .await
            .expect("update oauth login session");

        let updated_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load updated session")
            .expect("updated session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "callback-access".to_string(),
                refresh_token: "callback-refresh".to_string(),
                id_token: test_id_token(
                    "callback@example.com",
                    Some("org_callback"),
                    Some("user_callback"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: updated_session
                    .display_name
                    .clone()
                    .expect("display name should be stored"),
                session: updated_session.clone(),
                claims: test_claims(
                    "callback@example.com",
                    Some("org_callback"),
                    Some("user_callback"),
                ),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist oauth callback");

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load oauth account row")
            .expect("oauth account should exist");
        assert_eq!(account.display_name, "After Patch");
        assert_eq!(account.group_name.as_deref(), Some("new-group"));
        assert_eq!(account.note.as_deref(), Some("after note"));
        assert_eq!(account.is_mother, 1);

        let account_tag_ids = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(&state.pool)
        .await
        .expect("load oauth account tags");
        assert_eq!(account_tag_ids, vec![tag_id]);

        let group_note = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
        )
        .bind("new-group")
        .fetch_one(&state.pool)
        .await
        .expect("load group note");
        assert_eq!(group_note.as_deref(), Some("draft group note"));

        let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load completed session")
            .expect("completed session should exist");
        assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
        assert_eq!(completed_session.account_id, Some(account_id));
    }

    #[tokio::test]
    async fn update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let tag_id = insert_tag(&state.pool, "callback-race-sync", &test_tag_routing_rule())
            .await
            .expect("insert tag")
            .summary
            .id;

        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Race Before".to_string()),
                group_name: Some("race-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before note".to_string()),
                group_note: Some("before group note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load pending session")
            .expect("pending session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "race-access".to_string(),
                refresh_token: "race-refresh".to_string(),
                id_token: test_id_token(
                    "race@example.com",
                    Some("org_race"),
                    Some("user_race"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: pending_session
                    .display_name
                    .clone()
                    .expect("display name should be stored"),
                session: pending_session,
                claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist oauth callback");

        let mut repair_headers = HeaderMap::new();
        repair_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let repaired =
            update_oauth_login_session(
                State(state.clone()),
                repair_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Race After".to_string()),
                    group_name: OptionalField::Value("race-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("after note".to_string()),
                    group_note: OptionalField::Value("after group note".to_string()),
                    tag_ids: OptionalField::Value(vec![tag_id]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect("repair completed callback race")
            .0;

        assert_eq!(repaired.login_id, created.login_id);
        assert_eq!(repaired.status, LOGIN_SESSION_STATUS_COMPLETED);
        assert_eq!(repaired.account_id, Some(account_id));
        assert!(repaired.auth_url.is_none());
        assert!(repaired.redirect_uri.is_none());

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load repaired account row")
            .expect("oauth account should exist");
        assert_eq!(account.display_name, "Race After");
        assert_eq!(account.group_name.as_deref(), Some("race-group"));
        assert_eq!(account.note.as_deref(), Some("after note"));
        assert_eq!(account.is_mother, 1);

        let account_tag_ids = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(&state.pool)
        .await
        .expect("load repaired oauth account tags");
        assert_eq!(account_tag_ids, vec![tag_id]);

        let group_note = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
        )
        .bind("race-group")
        .fetch_one(&state.pool)
        .await
        .expect("load repaired group note");
        assert_eq!(group_note.as_deref(), Some("after group note"));

        let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("reload completed session")
            .expect("completed session should still exist");
        assert_ne!(completed_session.updated_at, created.updated_at);
        assert!(completed_session.consumed_at.is_some());

        let mut second_repair_headers = HeaderMap::new();
        second_repair_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&repaired.updated_at).expect("valid updated_at header"),
        );
        let second_repair = update_oauth_login_session(
            State(state.clone()),
            second_repair_headers,
            AxumPath(created.login_id.clone()),
            Json(UpdateOauthLoginSessionRequest {
                display_name: OptionalField::Value("Race Final".to_string()),
                group_name: OptionalField::Missing,
                group_bound_proxy_keys: OptionalField::Missing,
                note: OptionalField::Missing,
                group_note: OptionalField::Missing,
                tag_ids: OptionalField::Missing,
                is_mother: OptionalField::Missing,
                mailbox_session_id: OptionalField::Missing,
                mailbox_address: OptionalField::Missing,
            }),
        )
        .await
        .expect("repair completed callback race again with omitted fields")
        .0;

        let repaired_again = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load twice repaired account row")
            .expect("oauth account should still exist");
        assert_eq!(repaired_again.display_name, "Race Final");
        assert_eq!(repaired_again.group_name.as_deref(), Some("race-group"));
        assert_eq!(repaired_again.note.as_deref(), Some("after note"));
        assert_eq!(repaired_again.is_mother, 1);

        let account_tag_ids = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(&state.pool)
        .await
        .expect("load twice repaired oauth account tags");
        assert_eq!(account_tag_ids, vec![tag_id]);

        let second_group_note = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
        )
        .bind("race-group")
        .fetch_one(&state.pool)
        .await
        .expect("load twice repaired group note");
        assert_eq!(second_group_note.as_deref(), Some("after group note"));

        let repaired_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("reload repaired session after second patch")
            .expect("repaired session should still exist");
        assert_eq!(repaired_session.display_name.as_deref(), Some("Race Final"));
        assert_eq!(repaired_session.group_name.as_deref(), Some("race-group"));
        assert_eq!(repaired_session.note.as_deref(), Some("after note"));
        assert_eq!(
            parse_tag_ids_json(repaired_session.tag_ids_json.as_deref()),
            vec![tag_id]
        );
        assert_ne!(second_repair.updated_at, repaired.updated_at);
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_stale_completed_race_repairs() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Race Before".to_string()),
                group_name: Some("race-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before note".to_string()),
                group_note: Some("before group note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load pending session")
            .expect("pending session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "race-access".to_string(),
                refresh_token: "race-refresh".to_string(),
                id_token: test_id_token(
                    "race@example.com",
                    Some("org_race"),
                    Some("user_race"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: pending_session
                    .display_name
                    .clone()
                    .expect("display name should be stored"),
                session: pending_session,
                claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist oauth callback");

        let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load completed session")
            .expect("completed session should exist");
        assert_eq!(completed_session.updated_at, created.updated_at);
        assert!(completed_session.consumed_at.is_some());

        let mut first_headers = HeaderMap::new();
        first_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let first_repair =
            update_oauth_login_session(
                State(state.clone()),
                first_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Race Latest".to_string()),
                    group_name: OptionalField::Value("race-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("latest note".to_string()),
                    group_note: OptionalField::Value("latest group note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect("apply latest repair")
            .0;

        assert_ne!(first_repair.updated_at, created.updated_at);
        assert_eq!(first_repair.account_id, Some(account_id));

        let mut stale_headers = HeaderMap::new();
        stale_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let stale_err =
            update_oauth_login_session(
                State(state.clone()),
                stale_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Race Stale".to_string()),
                    group_name: OptionalField::Value("stale-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("stale note".to_string()),
                    group_note: OptionalField::Value("stale group note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(false),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect_err("reject stale repair");
        assert_eq!(stale_err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            stale_err.1,
            "This login session can no longer be edited.".to_string()
        );

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load account after stale repair rejection")
            .expect("oauth account should exist");
        assert_eq!(account.display_name, "Race Latest");
        assert_eq!(account.group_name.as_deref(), Some("race-group"));
        assert_eq!(account.note.as_deref(), Some("latest note"));
        assert_eq!(account.is_mother, 1);
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_completed_repairs_after_group_note_changes() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Race Before".to_string()),
                group_name: Some("race-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before note".to_string()),
                group_note: Some("before group note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load pending session")
            .expect("pending session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "race-access".to_string(),
                refresh_token: "race-refresh".to_string(),
                id_token: test_id_token(
                    "race@example.com",
                    Some("org_race"),
                    Some("user_race"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: pending_session
                    .display_name
                    .clone()
                    .expect("display name should be stored"),
                session: pending_session,
                claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist oauth callback");

        let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load completed session")
            .expect("completed session should exist");
        assert_eq!(completed_session.updated_at, created.updated_at);

        let mut conn = state.pool.acquire().await.expect("acquire group note conn");
        save_group_note_record_conn(
            &mut conn,
            "race-group",
            Some("manual latest group note".to_string()),
        )
        .await
        .expect("save manual latest group note");
        drop(conn);

        let mut repair_headers = HeaderMap::new();
        repair_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let repair_err =
            update_oauth_login_session(
                State(state.clone()),
                repair_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Race Latest".to_string()),
                    group_name: OptionalField::Value("race-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("latest note".to_string()),
                    group_note: OptionalField::Value("latest group note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect_err("reject repair after group note changes");
        assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            repair_err.1,
            "This login session can no longer be edited.".to_string()
        );

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load account after repair rejection")
            .expect("oauth account should exist");
        assert_eq!(account.display_name, "Race Before");
        assert_eq!(account.group_name.as_deref(), Some("race-group"));
        assert_eq!(account.note.as_deref(), Some("before note"));

        let group_note = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
        )
        .bind("race-group")
        .fetch_one(&state.pool)
        .await
        .expect("load preserved group note");
        assert_eq!(group_note.as_deref(), Some("manual latest group note"));
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_completed_repairs_after_account_changes() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let created = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Race Before".to_string()),
                group_name: Some("race-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: Some("before note".to_string()),
                group_note: Some("before group note".to_string()),
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create oauth login session")
        .0;

        let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load pending session")
            .expect("pending session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "race-access".to_string(),
                refresh_token: "race-refresh".to_string(),
                id_token: test_id_token(
                    "race@example.com",
                    Some("org_race"),
                    Some("user_race"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: pending_session
                    .display_name
                    .clone()
                    .expect("display name should be stored"),
                session: pending_session,
                claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist oauth callback");

        let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
            .await
            .expect("load completed session")
            .expect("completed session should exist");
        let consumed_at = completed_session
            .consumed_at
            .clone()
            .expect("completed session should record consumed_at");
        let newer_account_updated_at = next_login_session_updated_at(Some(consumed_at.as_str()));
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET display_name = ?2,
                note = ?3,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind("Manual Latest")
        .bind("manual latest note")
        .bind(&newer_account_updated_at)
        .execute(&state.pool)
        .await
        .expect("simulate newer account edit");

        let mut repair_headers = HeaderMap::new();
        repair_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
        );
        let repair_err =
            update_oauth_login_session(
                State(state.clone()),
                repair_headers,
                AxumPath(created.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Race Stale".to_string()),
                    group_name: OptionalField::Value("stale-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("stale note".to_string()),
                    group_note: OptionalField::Value("stale group note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(false),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect_err("reject completed repair after account changes");
        assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            repair_err.1,
            "This login session can no longer be edited.".to_string()
        );

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load account after rejecting stale completed repair")
            .expect("oauth account should exist");
        assert_eq!(account.display_name, "Manual Latest");
        assert_eq!(account.group_name.as_deref(), Some("race-group"));
        assert_eq!(account.note.as_deref(), Some("manual latest note"));
        assert_eq!(account.updated_at, newer_account_updated_at);
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_completed_failed_and_expired_sessions() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let update_payload = || UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Edited Session".to_string()),
            group_name: OptionalField::Value("edited-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            note: OptionalField::Value("edited note".to_string()),
            group_note: OptionalField::Value("edited group note".to_string()),
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        };

        let completed = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Completed Session".to_string()),
                group_name: Some("completed-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: None,
                group_note: None,
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create completed session seed")
        .0;
        sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
            .bind(&completed.login_id)
            .bind(LOGIN_SESSION_STATUS_COMPLETED)
            .execute(&state.pool)
            .await
            .expect("mark session completed");
        let completed_err = update_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(completed.login_id.clone()),
            Json(update_payload()),
        )
        .await
        .expect_err("completed session should reject edits");
        assert_eq!(completed_err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            completed_err.1,
            "This login session can no longer be edited."
        );

        let failed = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Failed Session".to_string()),
                group_name: Some("failed-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: None,
                group_note: None,
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create failed session seed")
        .0;
        sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
            .bind(&failed.login_id)
            .bind(LOGIN_SESSION_STATUS_FAILED)
            .execute(&state.pool)
            .await
            .expect("mark session failed");
        let failed_err = update_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(failed.login_id.clone()),
            Json(update_payload()),
        )
        .await
        .expect_err("failed session should reject edits");
        assert_eq!(failed_err.0, StatusCode::BAD_REQUEST);
        assert_eq!(failed_err.1, "This login session can no longer be edited.");

        let expired = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: Some("Expired Session".to_string()),
                group_name: Some("expired-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                note: None,
                group_note: None,
                account_id: None,
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create expired session seed")
        .0;
        sqlx::query("UPDATE pool_oauth_login_sessions SET expires_at = ?2 WHERE login_id = ?1")
            .bind(&expired.login_id)
            .bind("2020-01-01T00:00:00Z")
            .execute(&state.pool)
            .await
            .expect("expire session");
        let expired_err = update_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(expired.login_id.clone()),
            Json(update_payload()),
        )
        .await
        .expect_err("expired session should reject edits");
        assert_eq!(expired_err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            expired_err.1,
            "The login session has expired. Please create a new authorization link."
        );

        let expired_session = load_login_session_by_login_id(&state.pool, &expired.login_id)
            .await
            .expect("load expired session")
            .expect("expired session should exist");
        assert_eq!(expired_session.status, LOGIN_SESSION_STATUS_EXPIRED);
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_relogin_sessions() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
        let relogin = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                note: None,
                group_note: None,
                account_id: Some(account_id),
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create relogin session")
        .0;

        let err = update_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(relogin.login_id.clone()),
            Json(UpdateOauthLoginSessionRequest {
                display_name: OptionalField::Value("Edited Relogin".to_string()),
                group_name: OptionalField::Missing,
                group_bound_proxy_keys: OptionalField::Missing,
                note: OptionalField::Missing,
                group_note: OptionalField::Missing,
                tag_ids: OptionalField::Value(vec![]),
                is_mother: OptionalField::Value(false),
                mailbox_session_id: OptionalField::Missing,
                mailbox_address: OptionalField::Missing,
            }),
        )
        .await
        .expect_err("relogin session should reject edits");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.1,
            "This login session belongs to an existing account and cannot be edited."
        );
    }

    #[tokio::test]
    async fn update_oauth_login_session_rejects_completed_relogin_repairs() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
        let relogin = create_oauth_login_session(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateOauthLoginSessionRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                note: None,
                group_note: None,
                account_id: Some(account_id),
                tag_ids: vec![],
                is_mother: Some(false),
                mailbox_session_id: None,
                mailbox_address: None,
            }),
        )
        .await
        .expect("create relogin session")
        .0;

        let pending_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
            .await
            .expect("load relogin session")
            .expect("relogin session should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "relogin-access".to_string(),
                refresh_token: "relogin-refresh".to_string(),
                id_token: test_id_token(
                    "relogin@example.com",
                    Some("org_relogin"),
                    Some("user_relogin"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        let completed_account_id = persist_oauth_callback_inner(
            state.as_ref(),
            PersistOauthCallbackInput {
                display_name: "Relogin Target".to_string(),
                session: pending_session,
                claims: test_claims(
                    "relogin@example.com",
                    Some("org_relogin"),
                    Some("user_relogin"),
                ),
                encrypted_credentials,
                token_expires_at: "2026-04-01T00:00:00Z".to_string(),
            },
        )
        .await
        .expect("persist relogin callback");
        assert_eq!(completed_account_id, account_id);

        let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
            .await
            .expect("load completed relogin session")
            .expect("completed relogin session should exist");
        assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
        assert_eq!(
            completed_session.updated_at,
            completed_session.consumed_at.clone().unwrap()
        );

        let mut repair_headers = HeaderMap::new();
        repair_headers.insert(
            LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
            header::HeaderValue::from_str(&relogin.updated_at).expect("valid updated_at header"),
        );
        let err =
            update_oauth_login_session(
                State(state.clone()),
                repair_headers,
                AxumPath(relogin.login_id.clone()),
                Json(UpdateOauthLoginSessionRequest {
                    display_name: OptionalField::Value("Edited Relogin".to_string()),
                    group_name: OptionalField::Value("edited-group".to_string()),
                    group_bound_proxy_keys: OptionalField::Value(
                        test_required_group_bound_proxy_keys(),
                    ),
                    note: OptionalField::Value("edited note".to_string()),
                    group_note: OptionalField::Value("edited group note".to_string()),
                    tag_ids: OptionalField::Value(vec![]),
                    is_mother: OptionalField::Value(true),
                    mailbox_session_id: OptionalField::Missing,
                    mailbox_address: OptionalField::Missing,
                }),
            )
            .await
            .expect_err("completed relogin repair should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            err.1,
            "This login session can no longer be edited.".to_string()
        );

        let account = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load relogin target after rejected repair")
            .expect("relogin target should exist");
        assert_eq!(account.display_name, "Relogin Target");
        assert_ne!(account.group_name.as_deref(), Some("edited-group"));
        assert_ne!(account.note.as_deref(), Some("edited note"));
    }

    #[tokio::test]
    async fn upsert_oauth_account_preserves_route_cooldown_state_for_existing_account() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, "Cooldown OAuth Existing", None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Cooldown OAuth Existing",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims(
                    "cooldown-existing@example.com",
                    Some("cooldown_org"),
                    Some("cooldown_user"),
                ),
                encrypted_credentials: "encrypted-original".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("insert oauth account");
        tx.commit().await.expect("commit insert tx");

        seed_route_cooldown(
            &pool,
            account_id,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            300,
        )
        .await;
        let before = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row before update")
            .expect("row exists before update");

        let mut tx = pool.begin().await.expect("begin update tx");
        let updated_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: Some(account_id),
                display_name: "Cooldown OAuth Existing",
                group_name: None,
                is_mother: false,
                note: Some("updated note".to_string()),
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims(
                    "cooldown-existing@example.com",
                    Some("cooldown_org"),
                    Some("cooldown_user"),
                ),
                encrypted_credentials: "encrypted-updated".to_string(),
                token_expires_at: "2026-03-15T00:00:00Z",
            },
        )
        .await
        .expect("update oauth account");
        tx.commit().await.expect("commit update tx");

        assert_eq!(updated_id, account_id);
        let after = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row after update")
            .expect("row exists after update");

        assert_eq!(after.last_route_failure_at, before.last_route_failure_at);
        assert_eq!(
            after.last_route_failure_kind,
            before.last_route_failure_kind
        );
        assert_eq!(after.cooldown_until, before.cooldown_until);
        assert_eq!(
            after.consecutive_route_failures,
            before.consecutive_route_failures
        );
        assert_eq!(after.note.as_deref(), Some("updated note"));
        assert_eq!(
            after.encrypted_credentials.as_deref(),
            Some("encrypted-updated")
        );
    }

    #[tokio::test]
    async fn team_oauth_accounts_with_shared_account_id_are_not_flagged_as_duplicates() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx 1");
        ensure_display_name_available(&mut *tx, "First OAuth", None)
            .await
            .expect("first name available");
        let first_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "First OAuth",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("first@example.com", Some("org_shared"), Some("user_1")),
                encrypted_credentials: "encrypted-1".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("first oauth insert");
        tx.commit().await.expect("commit tx 1");

        let mut tx = pool.begin().await.expect("begin tx 2");
        ensure_display_name_available(&mut *tx, "Second OAuth", None)
            .await
            .expect("second name available");
        let second_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Second OAuth",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("second@example.com", Some("org_shared"), Some("user_2")),
                encrypted_credentials: "encrypted-2".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("second oauth insert");
        tx.commit().await.expect("commit tx 2");

        assert_ne!(first_id, second_id);
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pool_upstream_accounts WHERE kind = ?1",
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .fetch_one(&pool)
        .await
        .expect("count oauth rows");
        assert_eq!(count, 2);

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(duplicate_info.get(&first_id).is_none());
        assert!(duplicate_info.get(&second_id).is_none());

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| summary.id == first_id || summary.id == second_id)
                .all(|summary| summary.duplicate_info.is_none())
        );

        let first_detail = load_upstream_account_detail(&pool, first_id)
            .await
            .expect("load first detail")
            .expect("first detail exists");
        let second_detail = load_upstream_account_detail(&pool, second_id)
            .await
            .expect("load second detail")
            .expect("second detail exists");
        assert!(first_detail.summary.duplicate_info.is_none());
        assert!(second_detail.summary.duplicate_info.is_none());
    }

    #[tokio::test]
    async fn new_oauth_accounts_with_shared_user_id_are_preserved_and_flagged() {
        let pool = test_pool().await;

        for (display_name, email, account_id) in [
            ("First OAuth", "first@example.com", "org_1"),
            ("Second OAuth", "second@example.com", "org_2"),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims(email, Some(account_id), Some("user_shared")),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptUserId])
        );

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(summaries.iter().all(|summary| matches!(
                    summary.duplicate_info.as_ref().map(|info| info.reasons.as_slice()),
                    Some([DuplicateReason::SharedChatgptUserId])
                )));

        for summary in summaries {
            let detail = load_upstream_account_detail(&pool, summary.id)
                .await
                .expect("load detail")
                .expect("detail exists");
            assert!(matches!(
                detail
                    .summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptUserId])
            ));
        }
    }

    #[tokio::test]
    async fn mixed_plan_type_accounts_with_shared_account_id_remain_flagged() {
        let pool = test_pool().await;

        for (display_name, email, plan_type) in [
            ("Team OAuth", "team@example.com", Some("team")),
            ("Personal OAuth", "personal@example.com", Some("pro")),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(email, Some("org_shared"), None, plan_type),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
        );

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(summaries.iter().all(|summary| matches!(
                    summary.duplicate_info.as_ref().map(|info| info.reasons.as_slice()),
                    Some([DuplicateReason::SharedChatgptAccountId])
                )));

        for summary in summaries {
            let detail = load_upstream_account_detail(&pool, summary.id)
                .await
                .expect("load detail")
                .expect("detail exists");
            assert!(matches!(
                detail
                    .summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptAccountId])
            ));
        }
    }

    #[tokio::test]
    async fn latest_usage_sample_plan_type_clears_legacy_team_duplicate_flags() {
        let pool = test_pool().await;

        let mut inserted_ids = Vec::new();
        for (display_name, email, plan_type) in [
            ("Legacy Team One", "legacy-team-1@example.com", None),
            ("Legacy Team Two", "legacy-team-2@example.com", Some("pro")),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            let account_id = upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(
                        email,
                        Some("legacy_shared_org"),
                        None,
                        plan_type,
                    ),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
            inserted_ids.push(account_id);
        }

        for (index, account_id) in inserted_ids.iter().enumerate() {
            insert_limit_sample(
                &pool,
                *account_id,
                &format!("2026-03-15T00:00:0{}Z", index + 1),
                Some("team"),
            )
            .await;
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET plan_type_observed_at = '2026-03-14T00:00:00Z',
                    last_refreshed_at = '2026-03-14T00:00:00Z',
                    updated_at = '2026-03-14T00:00:00Z'
                WHERE id = ?1
                "#,
            )
            .bind(*account_id)
            .execute(&pool)
            .await
            .expect("age account claims");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(duplicate_info.is_empty());

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| inserted_ids.contains(&summary.id))
                .all(|summary| summary.plan_type.as_deref() == Some("team"))
        );

        for account_id in inserted_ids {
            let detail = load_upstream_account_detail(&pool, account_id)
                .await
                .expect("load detail")
                .expect("detail exists");
            assert_eq!(detail.summary.plan_type.as_deref(), Some("team"));
        }
    }

    #[tokio::test]
    async fn latest_usage_sample_plan_type_restores_non_team_duplicate_flags() {
        let pool = test_pool().await;

        let mut inserted_ids = Vec::new();
        for (display_name, email) in [
            ("Stale Team One", "stale-team-1@example.com"),
            ("Stale Team Two", "stale-team-2@example.com"),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            let account_id = upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(
                        email,
                        Some("stale_shared_org"),
                        None,
                        Some("team"),
                    ),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
            inserted_ids.push(account_id);
        }

        insert_limit_sample(&pool, inserted_ids[0], "2026-03-15T00:00:01Z", Some("team")).await;
        insert_limit_sample(&pool, inserted_ids[1], "2026-03-15T00:00:02Z", Some("pro")).await;

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
        );
    }

    #[tokio::test]
    async fn persist_usage_snapshot_uses_explicit_effective_plan_type() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, "Snapshot OAuth", None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Snapshot OAuth",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    "snapshot@example.com",
                    Some("snapshot_org"),
                    Some("snapshot_user"),
                    Some("team"),
                ),
                encrypted_credentials: "encrypted-snapshot".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");

        let snapshot = NormalizedUsageSnapshot {
            plan_type: None,
            limit_id: "gpt-4".to_string(),
            limit_name: Some("GPT-4".to_string()),
            primary: None,
            secondary: None,
            credits: None,
        };
        persist_usage_snapshot(&pool, account_id, Some("pro"), &snapshot, 30)
            .await
            .expect("persist snapshot");

        let stored_plan_type = sqlx::query_scalar::<_, Option<String>>(
            r#"
            SELECT plan_type
            FROM pool_upstream_account_limit_samples
            WHERE account_id = ?1
            ORDER BY captured_at DESC
            LIMIT 1
            "#,
        )
        .bind(account_id)
        .fetch_one(&pool)
        .await
        .expect("load sample plan type");
        assert_eq!(stored_plan_type.as_deref(), Some("pro"));
    }

    #[tokio::test]
    async fn refresh_without_plan_type_keeps_existing_plan_type_observed_at() {
        let pool = test_pool().await;
        let crypto_key = derive_secret_key("refresh-without-plan-type");

        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, "Refresh OAuth", None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Refresh OAuth",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    "refresh@example.com",
                    Some("refresh_org"),
                    Some("refresh_user"),
                    Some("team"),
                ),
                encrypted_credentials: encrypt_credentials(
                    &crypto_key,
                    &StoredCredentials::Oauth(StoredOauthCredentials {
                        access_token: "access-1".to_string(),
                        refresh_token: "refresh-1".to_string(),
                        id_token: test_id_token(
                            "refresh@example.com",
                            Some("refresh_org"),
                            Some("refresh_user"),
                            Some("team"),
                        ),
                        token_type: Some("Bearer".to_string()),
                    }),
                )
                .expect("encrypt oauth credentials"),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");

        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET plan_type_observed_at = '2026-03-15T00:00:01Z',
                last_refreshed_at = '2026-03-15T00:00:01Z',
                updated_at = '2026-03-15T00:00:01Z'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("seed observed_at");

        persist_oauth_credentials(
            &pool,
            account_id,
            &crypto_key,
            &StoredOauthCredentials {
                access_token: "access-2".to_string(),
                refresh_token: "refresh-2".to_string(),
                id_token: test_id_token(
                    "refresh@example.com",
                    Some("refresh_org"),
                    Some("refresh_user"),
                    None,
                ),
                token_type: Some("Bearer".to_string()),
            },
            "2026-03-16T00:00:00Z",
        )
        .await
        .expect("persist refreshed credentials");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row")
            .expect("row exists");
        assert_eq!(row.plan_type.as_deref(), Some("team"));
        assert_eq!(
            row.plan_type_observed_at.as_deref(),
            Some("2026-03-15T00:00:01Z")
        );
        assert!(row.last_refreshed_at.is_some());
        assert_ne!(
            row.last_refreshed_at.as_deref(),
            Some("2026-03-15T00:00:01Z")
        );
    }

    #[tokio::test]
    async fn snapshot_plan_type_fallback_prefers_latest_effective_sample() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, "Fallback OAuth", None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Fallback OAuth",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    "fallback@example.com",
                    Some("fallback_org"),
                    Some("fallback_user"),
                    Some("team"),
                ),
                encrypted_credentials: "encrypted-fallback".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");

        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET plan_type = 'team',
                plan_type_observed_at = '2026-03-15T00:00:01Z',
                last_refreshed_at = '2026-03-15T00:00:01Z'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("age account claims");
        insert_limit_sample(&pool, account_id, "2026-03-15T00:00:02Z", Some("pro")).await;

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row")
            .expect("row exists");
        let snapshot = NormalizedUsageSnapshot {
            plan_type: None,
            limit_id: "gpt-4".to_string(),
            limit_name: Some("GPT-4".to_string()),
            primary: None,
            secondary: None,
            credits: None,
        };

        let effective_plan_type = resolve_snapshot_plan_type(&pool, &row, &snapshot)
            .await
            .expect("resolve snapshot plan type");
        assert_eq!(effective_plan_type.as_deref(), Some("pro"));
    }

    #[tokio::test]
    async fn snapshot_plan_type_fallback_prefers_refreshed_claims_over_stale_non_empty_sample() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, "Refreshed Fallback OAuth", None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Refreshed Fallback OAuth",
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    "refreshed-fallback@example.com",
                    Some("refreshed_fallback_org"),
                    Some("refreshed_fallback_user"),
                    Some("team"),
                ),
                encrypted_credentials: "encrypted-refreshed-fallback".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");

        insert_limit_sample(&pool, account_id, "2026-03-15T00:00:01Z", Some("team")).await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET plan_type = 'pro',
                plan_type_observed_at = '2026-03-15T00:00:02Z',
                last_refreshed_at = '2026-03-15T00:00:02Z'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .execute(&pool)
        .await
        .expect("refresh account claims");

        let row = load_upstream_account_row(&pool, account_id)
            .await
            .expect("load row")
            .expect("row exists");
        let snapshot = NormalizedUsageSnapshot {
            plan_type: None,
            limit_id: "gpt-4".to_string(),
            limit_name: Some("GPT-4".to_string()),
            primary: None,
            secondary: None,
            credits: None,
        };

        let effective_plan_type = resolve_snapshot_plan_type(&pool, &row, &snapshot)
            .await
            .expect("resolve snapshot plan type");
        assert_eq!(effective_plan_type.as_deref(), Some("pro"));
    }

    #[tokio::test]
    async fn fresher_account_claims_override_stale_non_empty_samples() {
        let pool = test_pool().await;

        let mut inserted_ids = Vec::new();
        for (display_name, email) in [
            ("Refreshed Team One", "refreshed-team-1@example.com"),
            ("Refreshed Team Two", "refreshed-team-2@example.com"),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            let account_id = upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(
                        email,
                        Some("refreshed_shared_org"),
                        None,
                        Some("team"),
                    ),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
            inserted_ids.push(account_id);
        }

        for account_id in &inserted_ids {
            insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:01Z", Some("team")).await;
            insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:02Z", None).await;
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'pro',
                    plan_type_observed_at = '2026-03-15T00:00:03Z',
                    last_refreshed_at = '2026-03-15T00:00:03Z',
                    updated_at = '2026-03-15T00:00:03Z'
                WHERE id = ?1
                "#,
            )
            .bind(*account_id)
            .execute(&pool)
            .await
            .expect("refresh account claims");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
        );

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| inserted_ids.contains(&summary.id))
                .all(|summary| summary.plan_type.as_deref() == Some("pro"))
        );

        for account_id in inserted_ids {
            let detail = load_upstream_account_detail(&pool, account_id)
                .await
                .expect("load detail")
                .expect("detail exists");
            assert_eq!(detail.summary.plan_type.as_deref(), Some("pro"));
        }
    }

    #[tokio::test]
    async fn refreshed_claims_override_older_non_empty_samples_without_newer_plan_samples() {
        let pool = test_pool().await;

        let mut inserted_ids = Vec::new();
        for (display_name, email) in [
            ("Claims Fresh One", "claims-fresh-1@example.com"),
            ("Claims Fresh Two", "claims-fresh-2@example.com"),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            let account_id = upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(
                        email,
                        Some("claims_fresh_shared_org"),
                        None,
                        Some("team"),
                    ),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
            inserted_ids.push(account_id);
        }

        for account_id in &inserted_ids {
            insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:01Z", Some("team")).await;
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'pro',
                    plan_type_observed_at = '2026-03-15T00:00:02Z',
                    last_refreshed_at = '2026-03-15T00:00:02Z',
                    updated_at = '2026-03-15T00:00:03Z'
                WHERE id = ?1
                "#,
            )
            .bind(*account_id)
            .execute(&pool)
            .await
            .expect("refresh account claims");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
        );

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| inserted_ids.contains(&summary.id))
                .all(|summary| summary.plan_type.as_deref() == Some("pro"))
        );
    }

    #[tokio::test]
    async fn same_second_refreshed_claims_win_against_latest_non_empty_sample() {
        let pool = test_pool().await;

        let mut inserted_ids = Vec::new();
        for (display_name, email) in [
            ("Same Second One", "same-second-1@example.com"),
            ("Same Second Two", "same-second-2@example.com"),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            let account_id = upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(
                        email,
                        Some("same_second_org"),
                        None,
                        Some("team"),
                    ),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
            inserted_ids.push(account_id);
        }

        for account_id in &inserted_ids {
            insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:02Z", Some("team")).await;
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'pro',
                    plan_type_observed_at = '2026-03-15T00:00:02Z',
                    last_refreshed_at = '2026-03-15T00:00:02Z',
                    updated_at = '2026-03-15T00:00:02Z'
                WHERE id = ?1
                "#,
            )
            .bind(*account_id)
            .execute(&pool)
            .await
            .expect("seed same-second claims");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
        );

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| inserted_ids.contains(&summary.id))
                .all(|summary| summary.plan_type.as_deref() == Some("pro"))
        );
    }

    #[tokio::test]
    async fn metadata_updates_do_not_override_newer_usage_sample_plan_type() {
        let pool = test_pool().await;

        let mut inserted_ids = Vec::new();
        for (display_name, email) in [
            ("Sample Fresh One", "sample-fresh-1@example.com"),
            ("Sample Fresh Two", "sample-fresh-2@example.com"),
        ] {
            let mut tx = pool.begin().await.expect("begin tx");
            ensure_display_name_available(&mut *tx, display_name, None)
                .await
                .expect("name available");
            let account_id = upsert_oauth_account(
                &mut tx,
                OauthAccountUpsert {
                    account_id: None,
                    display_name,
                    group_name: None,
                    is_mother: false,
                    note: None,
                    tag_ids: vec![],
                    requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                    claims: &test_claims_with_plan_type(
                        email,
                        Some("sample_fresh_shared_org"),
                        None,
                        Some("team"),
                    ),
                    encrypted_credentials: format!("encrypted-{display_name}"),
                    token_expires_at: "2026-03-14T00:00:00Z",
                },
            )
            .await
            .expect("oauth insert");
            tx.commit().await.expect("commit tx");
            inserted_ids.push(account_id);
        }

        for account_id in &inserted_ids {
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET plan_type = 'team',
                    plan_type_observed_at = '2026-03-15T00:00:01Z',
                    last_refreshed_at = '2026-03-15T00:00:01Z',
                    updated_at = '2026-03-15T00:00:01Z'
                WHERE id = ?1
                "#,
            )
            .bind(*account_id)
            .execute(&pool)
            .await
            .expect("seed account claims");
            insert_limit_sample(&pool, *account_id, "2026-03-15T00:00:02Z", Some("pro")).await;
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    updated_at = '2026-03-15T00:00:03Z'
                WHERE id = ?1
                "#,
            )
            .bind(*account_id)
            .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
            .execute(&pool)
            .await
            .expect("simulate metadata update");
        }

        let duplicate_info = load_duplicate_info_map(&pool)
            .await
            .expect("load duplicate info");
        assert!(
            duplicate_info
                .values()
                .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
        );

        let summaries = load_upstream_account_summaries(&pool)
            .await
            .expect("load summaries");
        assert!(
            summaries
                .iter()
                .filter(|summary| inserted_ids.contains(&summary.id))
                .all(|summary| summary.plan_type.as_deref() == Some("pro"))
        );
    }

    #[tokio::test]
    async fn relink_updates_existing_oauth_row_without_inserting() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.expect("begin tx");
        let original_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name: "Original OAuth",
                group_name: Some("prod".to_string()),
                is_mother: false,
                note: Some("note".to_string()),
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("first@example.com", Some("org_shared"), Some("user_1")),
                encrypted_credentials: "encrypted-1".to_string(),
                token_expires_at: "2026-03-14T00:00:00Z",
            },
        )
        .await
        .expect("insert original oauth");
        tx.commit().await.expect("commit tx");

        let mut tx = pool.begin().await.expect("begin relink tx");
        ensure_display_name_available(&mut *tx, "Renamed OAuth", Some(original_id))
            .await
            .expect("name available");
        let relinked_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: Some(original_id),
                display_name: "Renamed OAuth",
                group_name: Some("prod".to_string()),
                is_mother: false,
                note: Some("fresh".to_string()),
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims("second@example.com", Some("org_shared"), Some("user_9")),
                encrypted_credentials: "encrypted-2".to_string(),
                token_expires_at: "2026-03-15T00:00:00Z",
            },
        )
        .await
        .expect("relink oauth");
        tx.commit().await.expect("commit relink tx");

        assert_eq!(relinked_id, original_id);
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_accounts")
            .fetch_one(&pool)
            .await
            .expect("count accounts");
        assert_eq!(count, 1);

        let renamed = load_upstream_account_row(&pool, original_id)
            .await
            .expect("load updated row")
            .expect("row exists");
        assert_eq!(renamed.display_name, "Renamed OAuth");
        assert_eq!(renamed.chatgpt_user_id.as_deref(), Some("user_9"));
    }

    #[tokio::test]
    async fn display_name_uniqueness_is_case_insensitive_and_self_excluding() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, " Alpha ").await;

        let mut tx = pool.begin().await.expect("begin tx conflict");
        let conflict = ensure_display_name_available(&mut *tx, "alpha", None).await;
        assert_eq!(
            conflict,
            Err((
                StatusCode::CONFLICT,
                "displayName must be unique".to_string()
            ))
        );

        let allowed = ensure_display_name_available(&mut *tx, " alpha ", Some(account_id)).await;
        assert!(allowed.is_ok());
    }

    #[test]
    fn parse_mailbox_code_prefers_subject_match() {
        let detail = MoeMailMessageDetail {
            id: "msg_1".to_string(),
            subject: Some("Your ChatGPT code is 612345".to_string()),
            content: Some("Ignore body 000000".to_string()),
            html: None,
            received_at: Some("2026-03-16T00:00:00Z".to_string()),
        };

        let parsed = parse_mailbox_code(&detail).expect("subject code");
        assert_eq!(parsed.value, "612345");
        assert_eq!(parsed.source, "subject");
    }

    #[test]
    fn parse_mailbox_code_falls_back_to_body_match() {
        let detail = MoeMailMessageDetail {
            id: "msg_2".to_string(),
            subject: Some("Security notice".to_string()),
            content: Some("Use this verification code: 481122 to continue.".to_string()),
            html: None,
            received_at: Some("2026-03-16T00:00:00Z".to_string()),
        };

        let parsed = parse_mailbox_code(&detail).expect("body code");
        assert_eq!(parsed.value, "481122");
        assert_eq!(parsed.source, "content");
    }

    #[test]
    fn parse_mailbox_code_supports_localized_subjects() {
        let detail = MoeMailMessageDetail {
            id: "msg_zh_subject".to_string(),
            subject: Some("你的 OpenAI 代码为 438211".to_string()),
            content: Some("如果这不是你本人操作，请重置密码。".to_string()),
            html: None,
            received_at: Some("2026-03-23T23:48:33Z".to_string()),
        };

        let parsed = parse_mailbox_code(&detail).expect("localized subject code");
        assert_eq!(parsed.value, "438211");
        assert_eq!(parsed.source, "subject");
    }

    #[test]
    fn parse_mailbox_code_supports_localized_html_and_fullwidth_digits() {
        let detail = MoeMailMessageDetail {
            id: "msg_zh_html".to_string(),
            subject: Some("安全提醒".to_string()),
            content: None,
            html: Some(
                "<div>OpenAI</div><p>输入此临时验证码以继续：</p><strong>４３８２１１</strong>"
                    .to_string(),
            ),
            received_at: Some("2026-03-24T00:00:00Z".to_string()),
        };

        let parsed = parse_mailbox_code(&detail).expect("localized html code");
        assert_eq!(parsed.value, "438211");
        assert_eq!(parsed.source, "html");
    }

    #[test]
    fn parse_mailbox_code_prefers_digits_after_marker() {
        let detail = MoeMailMessageDetail {
            id: "msg_order_and_code".to_string(),
            subject: Some("OpenAI order update".to_string()),
            content: Some("Order 1234. Your verification code is 567890.".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:05:30Z".to_string()),
        };

        let parsed = parse_mailbox_code(&detail).expect("verification code");
        assert_eq!(parsed.value, "567890");
        assert_eq!(parsed.source, "content");
    }

    #[test]
    fn parse_mailbox_code_rejects_weak_subject_match_without_local_brand() {
        let detail = MoeMailMessageDetail {
            id: "msg_weak_subject_without_local_brand".to_string(),
            subject: Some("Your code is 123456".to_string()),
            content: Some("OpenAI account activity summary".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:05:45Z".to_string()),
        };

        assert!(parse_mailbox_code(&detail).is_none());
    }

    #[test]
    fn parse_mailbox_code_rejects_strong_subject_match_without_brand() {
        let detail = MoeMailMessageDetail {
            id: "msg_strong_subject_without_brand".to_string(),
            subject: Some("验证码 123456".to_string()),
            content: Some("请在十分钟内完成验证。".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:05:50Z".to_string()),
        };

        assert!(parse_mailbox_code(&detail).is_none());
    }

    #[test]
    fn parse_mailbox_code_rejects_unrelated_numbers_without_code_semantics() {
        let detail = MoeMailMessageDetail {
            id: "msg_negative_code".to_string(),
            subject: Some("OpenAI receipt 438211".to_string()),
            content: Some("Invoice total: 23.00 USD".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:05:00Z".to_string()),
        };

        assert!(parse_mailbox_code(&detail).is_none());
    }

    #[test]
    fn parse_mailbox_invite_extracts_workspace_link() {
        let detail = MoeMailMessageDetail {
            id: "msg_3".to_string(),
            subject: Some("Alex has invited you to a workspace".to_string()),
            content: Some(
                "Join workspace: https://chatgpt.com/workspace/invite/abc123".to_string(),
            ),
            html: None,
            received_at: Some("2026-03-16T00:00:00Z".to_string()),
        };

        let parsed = parse_mailbox_invite(&detail).expect("invite summary");
        assert_eq!(parsed.subject, "Alex has invited you to a workspace");
        assert_eq!(
            parsed.copy_value,
            "https://chatgpt.com/workspace/invite/abc123"
        );
        assert_eq!(parsed.copy_label, "invite-link");
    }

    #[test]
    fn parse_mailbox_invite_supports_localized_templates() {
        let detail = MoeMailMessageDetail {
            id: "msg_zh_invite".to_string(),
            subject: Some("Alice 邀请你加入 OpenAI 工作区".to_string()),
            content: Some("请接受邀请：https://chatgpt.com/workspace/invite/abc123".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:06:00Z".to_string()),
        };

        let parsed = parse_mailbox_invite(&detail).expect("localized invite");
        assert_eq!(parsed.subject, "Alice 邀请你加入 OpenAI 工作区");
        assert_eq!(
            parsed.copy_value,
            "https://chatgpt.com/workspace/invite/abc123"
        );
    }

    #[test]
    fn parse_mailbox_invite_accepts_body_only_workspace_invites() {
        let detail = MoeMailMessageDetail {
            id: "msg_body_only_invite".to_string(),
            subject: Some("OpenAI workspace update".to_string()),
            content: Some(
                "请接受邀请并加入工作区：https://chatgpt.com/workspace/invite/accept?workspace=ws_789"
                    .to_string(),
            ),
            html: None,
            received_at: Some("2026-03-24T00:06:30Z".to_string()),
        };

        let parsed = parse_mailbox_invite(&detail).expect("body invite");
        assert_eq!(
            parsed.copy_value,
            "https://chatgpt.com/workspace/invite/accept?workspace=ws_789"
        );
    }

    #[test]
    fn parse_mailbox_invite_accepts_query_driven_cta_links() {
        let detail = MoeMailMessageDetail {
            id: "msg_query_invite".to_string(),
            subject: Some("Alice has invited you to a workspace".to_string()),
            content: Some(
                "Open your invite: https://chatgpt.com/workspace?invite=abc123".to_string(),
            ),
            html: None,
            received_at: Some("2026-03-24T00:06:45Z".to_string()),
        };

        let parsed = parse_mailbox_invite(&detail).expect("query invite");
        assert_eq!(
            parsed.copy_value,
            "https://chatgpt.com/workspace?invite=abc123"
        );
    }

    #[test]
    fn parse_mailbox_invite_accepts_body_only_invites_without_workspace_keyword() {
        let detail = MoeMailMessageDetail {
            id: "msg_body_only_plain_invite".to_string(),
            subject: Some("OpenAI account notice".to_string()),
            content: Some("Accept invitation: https://chatgpt.com/invite/abc123".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:06:50Z".to_string()),
        };

        let parsed = parse_mailbox_invite(&detail).expect("body invite without workspace");
        assert_eq!(parsed.copy_value, "https://chatgpt.com/invite/abc123");
    }

    #[test]
    fn parse_mailbox_invite_accepts_redirect_wrapped_brand_invites() {
        let detail = MoeMailMessageDetail {
            id: "msg_redirect_wrapped_invite".to_string(),
            subject: Some("Alex has invited you to a workspace".to_string()),
            content: Some(
                "Accept invitation: https://click.example.com/track?target=https%3A%2F%2Fchatgpt.com%2Fworkspace%2Finvite%2Fabc123".to_string(),
            ),
            html: None,
            received_at: Some("2026-03-24T00:07:10Z".to_string()),
        };

        let parsed = parse_mailbox_invite(&detail).expect("redirect wrapped invite");
        assert_eq!(
            parsed.copy_value,
            "https://chatgpt.com/workspace/invite/abc123"
        );
    }

    #[test]
    fn parse_mailbox_invite_rejects_non_invite_workspace_links() {
        let detail = MoeMailMessageDetail {
            id: "msg_negative_invite".to_string(),
            subject: Some("OpenAI workspace digest".to_string()),
            content: Some("Workspace docs: https://chatgpt.com/workspace".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:07:00Z".to_string()),
        };

        assert!(parse_mailbox_invite(&detail).is_none());
    }

    #[test]
    fn parse_mailbox_invite_rejects_help_articles_about_accepting_invites() {
        let detail = MoeMailMessageDetail {
            id: "msg_help_article".to_string(),
            subject: Some("OpenAI workspace help".to_string()),
            content: Some(
                "Need help to accept invitation to your workspace? Read https://help.openai.com/en/articles/12345-accept-invitation-to-workspace"
                    .to_string(),
            ),
            html: None,
            received_at: Some("2026-03-24T00:07:30Z".to_string()),
        };

        assert!(parse_mailbox_invite(&detail).is_none());
    }

    #[test]
    fn parse_mailbox_invite_rejects_generic_workspace_url_even_with_invite_subject() {
        let detail = MoeMailMessageDetail {
            id: "msg_negative_workspace_home".to_string(),
            subject: Some("Alice has invited you to a workspace".to_string()),
            content: Some("Open workspace: https://chatgpt.com/workspace".to_string()),
            html: None,
            received_at: Some("2026-03-24T00:08:00Z".to_string()),
        };

        assert!(parse_mailbox_invite(&detail).is_none());
    }

    #[test]
    fn normalize_mailbox_text_converts_fullwidth_digits_and_collapses_whitespace() {
        assert_eq!(
            normalize_mailbox_text("  OpenAI　验证码：４３８２１１ \n 下一步  "),
            "openai 验证码:438211 下一步"
        );
    }

    #[test]
    fn validate_mailbox_binding_fields_requires_complete_pair() {
        assert!(validate_mailbox_binding_fields(None, None).is_ok());
        assert!(
            validate_mailbox_binding_fields(Some("session_1"), Some("mail@example.com")).is_ok()
        );
        assert!(validate_mailbox_binding_fields(Some("session_1"), None).is_err());
        assert!(validate_mailbox_binding_fields(None, Some("mail@example.com")).is_err());
    }

    #[test]
    fn normalize_mailbox_address_trims_and_lowercases() {
        assert_eq!(
            normalize_mailbox_address("  Mixed.Case+1@Example.COM "),
            Some("mixed.case+1@example.com".to_string())
        );
        assert_eq!(normalize_mailbox_address("   "), None);
    }

    #[test]
    fn normalize_mailbox_domain_accepts_common_moemail_variants() {
        assert_eq!(
            normalize_mailbox_domain("MAIL-TW.707079.XYZ"),
            Some("mail-tw.707079.xyz".to_string())
        );
        assert_eq!(
            normalize_mailbox_domain("@mail-tw.707079.xyz"),
            Some("mail-tw.707079.xyz".to_string())
        );
        assert_eq!(
            normalize_mailbox_domain("finance.lab.d5r@mail-tw.707079.xyz"),
            Some("mail-tw.707079.xyz".to_string())
        );
        assert_eq!(normalize_mailbox_domain("   "), None);
    }

    #[test]
    fn moemail_supported_domains_normalize_config_tokens() {
        let payload = MoeMailConfigPayload {
            email_domains: Some(
                "mail-tw.707079.xyz, @MAIL-US.707079.XYZ ; finance.lab.d5r@mail-eu.707079.xyz"
                    .to_string(),
            ),
        };
        let domains = moemail_supported_domains(&payload);
        assert!(domains.contains("mail-tw.707079.xyz"));
        assert!(domains.contains("mail-us.707079.xyz"));
        assert!(domains.contains("mail-eu.707079.xyz"));
    }

    #[test]
    fn requested_manual_mailbox_address_distinguishes_missing_from_blank_input() {
        assert!(matches!(
            requested_manual_mailbox_address(None),
            RequestedManualMailboxAddress::Missing
        ));
        assert_eq!(
            requested_manual_mailbox_address(Some("  Mixed.Case@Example.COM  ")),
            RequestedManualMailboxAddress::Valid("mixed.case@example.com".to_string())
        );
        assert_eq!(
            requested_manual_mailbox_address(Some("   ")),
            RequestedManualMailboxAddress::Invalid("   ".to_string())
        );
    }

    #[test]
    fn mailbox_address_is_valid_rejects_broken_values() {
        assert!(mailbox_address_is_valid("valid.user@example.com"));
        assert!(!mailbox_address_is_valid("broken-address"));
        assert!(!mailbox_address_is_valid("missing-domain@"));
    }

    #[test]
    fn mailbox_addresses_match_normalizes_case_and_whitespace() {
        assert!(mailbox_addresses_match(
            Some(" Manual.User@Example.com "),
            Some("manual.user@example.com")
        ));
        assert!(!mailbox_addresses_match(
            Some("one@example.com"),
            Some("two@example.com")
        ));
    }

    #[test]
    fn normalize_mailbox_session_expires_at_converts_rfc3339_offsets_to_utc_iso() {
        assert_eq!(
            normalize_mailbox_session_expires_at(
                Some("2026-03-18T10:00:00+08:00"),
                Utc.with_ymd_and_hms(2026, 3, 17, 0, 0, 0).unwrap(),
            ),
            "2026-03-18T02:00:00Z"
        );
    }

    #[test]
    fn normalize_mailbox_session_expires_at_falls_back_when_source_is_invalid() {
        let fallback = Utc.with_ymd_and_hms(2026, 3, 17, 8, 9, 10).unwrap();
        assert_eq!(
            normalize_mailbox_session_expires_at(Some("not-a-timestamp"), fallback),
            "2026-03-17T08:09:10Z"
        );
    }

    #[test]
    fn expired_mailbox_session_requires_remote_delete_skips_attached_mailboxes() {
        let attached = OauthMailboxSessionRow {
            session_id: "session_attached".to_string(),
            remote_email_id: "email_attached".to_string(),
            email_address: "attached@example.com".to_string(),
            email_domain: "example.com".to_string(),
            mailbox_source: Some(OAUTH_MAILBOX_SOURCE_ATTACHED.to_string()),
            latest_code_value: None,
            latest_code_source: None,
            latest_code_updated_at: None,
            invite_subject: None,
            invite_copy_value: None,
            invite_copy_label: None,
            invite_updated_at: None,
            invited: 0,
            last_message_id: None,
            created_at: "2026-03-17T00:00:00Z".to_string(),
            updated_at: "2026-03-17T00:00:00Z".to_string(),
            expires_at: "2026-03-17T00:10:00Z".to_string(),
        };
        let generated = OauthMailboxSessionRow {
            mailbox_source: Some(OAUTH_MAILBOX_SOURCE_GENERATED.to_string()),
            ..attached.clone()
        };

        assert!(!expired_mailbox_session_requires_remote_delete(&attached));
        assert!(expired_mailbox_session_requires_remote_delete(&generated));
    }

    #[test]
    fn moemail_attach_status_is_not_readable_only_for_permission_and_missing() {
        assert!(moemail_attach_status_is_not_readable(
            reqwest::StatusCode::FORBIDDEN
        ));
        assert!(moemail_attach_status_is_not_readable(
            reqwest::StatusCode::NOT_FOUND
        ));
        assert!(!moemail_attach_status_is_not_readable(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!moemail_attach_status_is_not_readable(
            reqwest::StatusCode::GATEWAY_TIMEOUT
        ));
    }

    #[tokio::test]
    async fn create_oauth_mailbox_session_accepts_supported_domain_variants_for_existing_mailbox() {
        let harness = spawn_moemail_test_harness(
            "@MAIL-TW.707079.XYZ, mail-us.707079.xyz",
            vec![(
                "email_existing".to_string(),
                "finance.lab.d5r@mail-tw.707079.xyz".to_string(),
                Some("2026-03-20T12:50:00.000Z".to_string()),
            )],
        )
        .await;
        let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
            "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
        }))
        .expect("deserialize mailbox request");

        let Json(response) = create_oauth_mailbox_session(
            State(harness.state.clone()),
            HeaderMap::new(),
            Json(payload),
        )
        .await
        .expect("create mailbox session");

        assert!(response.supported);
        assert_eq!(response.email_address, "finance.lab.d5r@mail-tw.707079.xyz");
        assert_eq!(
            response.source.as_deref(),
            Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
        );
        let session_id = response.session_id.expect("session id");
        let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
            .await
            .expect("load mailbox session")
            .expect("stored mailbox session");
        assert_eq!(
            row.mailbox_source.as_deref(),
            Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
        );
        assert!(
            harness.stub.generated_requests.lock().await.is_empty(),
            "existing readable mailbox should not be recreated"
        );

        harness.abort();
    }

    #[tokio::test]
    async fn create_oauth_mailbox_session_creates_missing_supported_mailbox() {
        let harness = spawn_moemail_test_harness("@mail-tw.707079.xyz", Vec::new()).await;
        let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
            "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
        }))
        .expect("deserialize mailbox request");

        let Json(response) = create_oauth_mailbox_session(
            State(harness.state.clone()),
            HeaderMap::new(),
            Json(payload),
        )
        .await
        .expect("create mailbox session");

        assert!(response.supported);
        assert_eq!(response.email_address, "finance.lab.d5r@mail-tw.707079.xyz");
        assert_eq!(
            response.source.as_deref(),
            Some(OAUTH_MAILBOX_SOURCE_GENERATED)
        );
        let generated_requests = harness.stub.generated_requests.lock().await.clone();
        assert_eq!(
            generated_requests,
            vec![(
                "finance.lab.d5r".to_string(),
                "mail-tw.707079.xyz".to_string()
            )]
        );
        let session_id = response.session_id.expect("session id");
        let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
            .await
            .expect("load mailbox session")
            .expect("stored mailbox session");
        assert_eq!(
            row.mailbox_source.as_deref(),
            Some(OAUTH_MAILBOX_SOURCE_GENERATED)
        );
        assert_eq!(row.email_address, "finance.lab.d5r@mail-tw.707079.xyz");

        harness.abort();
    }

    #[tokio::test]
    async fn create_oauth_mailbox_session_rejects_true_unsupported_domains() {
        let harness = spawn_moemail_test_harness("mail-us.707079.xyz", Vec::new()).await;
        let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
            "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
        }))
        .expect("deserialize mailbox request");

        let Json(response) = create_oauth_mailbox_session(
            State(harness.state.clone()),
            HeaderMap::new(),
            Json(payload),
        )
        .await
        .expect("create mailbox session");

        assert!(!response.supported);
        assert_eq!(response.reason.as_deref(), Some("unsupported_domain"));
        assert!(
            harness.stub.generated_requests.lock().await.is_empty(),
            "unsupported domains must not trigger remote mailbox creation"
        );

        harness.abort();
    }

    #[tokio::test]
    async fn delete_oauth_mailbox_session_deletes_remote_for_generated_manual_mailbox() {
        let harness = spawn_moemail_test_harness("@mail-tw.707079.xyz", Vec::new()).await;
        let payload: CreateOauthMailboxSessionRequest = serde_json::from_value(json!({
            "emailAddress": "finance.lab.d5r@mail-tw.707079.xyz"
        }))
        .expect("deserialize mailbox request");
        let Json(created) = create_oauth_mailbox_session(
            State(harness.state.clone()),
            HeaderMap::new(),
            Json(payload),
        )
        .await
        .expect("create mailbox session");
        let session_id = created.session_id.expect("session id");
        let row = load_oauth_mailbox_session(&harness.state.pool, &session_id)
            .await
            .expect("load mailbox session")
            .expect("stored mailbox session");

        let status = delete_oauth_mailbox_session(
            State(harness.state.clone()),
            HeaderMap::new(),
            AxumPath(session_id.clone()),
        )
        .await
        .expect("delete mailbox session");

        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            harness.stub.deleted_ids.lock().await.clone(),
            vec![row.remote_email_id]
        );
        assert!(
            load_oauth_mailbox_session(&harness.state.pool, &session_id)
                .await
                .expect("load mailbox session after delete")
                .is_none()
        );

        harness.abort();
    }

    #[tokio::test]
    async fn cleanup_expired_oauth_mailbox_sessions_deletes_remote_for_generated_manual_mailbox() {
        let harness = spawn_moemail_test_harness(
            "@mail-tw.707079.xyz",
            vec![(
                "generated_1".to_string(),
                "finance.lab.d5r@mail-tw.707079.xyz".to_string(),
                None,
            )],
        )
        .await;
        sqlx::query(
            r#"
            INSERT INTO pool_oauth_mailbox_sessions (
                session_id, remote_email_id, email_address, email_domain, mailbox_source,
                latest_code_value, latest_code_source, latest_code_updated_at, invite_subject,
                invite_copy_value, invite_copy_label, invite_updated_at, invited, last_message_id,
                created_at, updated_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, NULL, ?6, ?6, ?7)
            "#,
        )
        .bind("expired_manual_generated")
        .bind("generated_1")
        .bind("finance.lab.d5r@mail-tw.707079.xyz")
        .bind("mail-tw.707079.xyz")
        .bind(OAUTH_MAILBOX_SOURCE_GENERATED)
        .bind("2026-03-17T00:00:00Z")
        .bind("2026-03-17T00:01:00Z")
        .execute(&harness.state.pool)
        .await
        .expect("insert expired mailbox session");

        cleanup_expired_oauth_mailbox_sessions(harness.state.as_ref())
            .await
            .expect("cleanup expired mailbox sessions");

        assert_eq!(
            harness.stub.deleted_ids.lock().await.clone(),
            vec!["generated_1".to_string()]
        );
        assert!(
            load_oauth_mailbox_session(&harness.state.pool, "expired_manual_generated")
                .await
                .expect("load cleaned mailbox session")
                .is_none()
        );

        harness.abort();
    }

    #[test]
    fn collect_unseen_mailbox_messages_stops_at_last_seen_id() {
        let messages = vec![
            MoeMailMessageSummary {
                id: "msg_3".to_string(),
                subject: Some("newest".to_string()),
                received_at: Some("2026-03-16T03:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
                id: "msg_2".to_string(),
                subject: Some("baseline".to_string()),
                received_at: Some("2026-03-16T02:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
                id: "msg_1".to_string(),
                subject: Some("older".to_string()),
                received_at: Some("2026-03-16T01:00:00Z".to_string()),
            },
        ];

        let unseen = collect_unseen_mailbox_messages(messages, Some("msg_2"));

        assert_eq!(unseen.len(), 1);
        assert_eq!(unseen[0].id, "msg_3");
    }

    #[test]
    fn collect_unseen_mailbox_messages_keeps_all_when_baseline_is_missing() {
        let messages = vec![
            MoeMailMessageSummary {
                id: "msg_2".to_string(),
                subject: None,
                received_at: Some("2026-03-16T02:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
                id: "msg_1".to_string(),
                subject: None,
                received_at: Some("2026-03-16T01:00:00Z".to_string()),
            },
        ];

        let unseen = collect_unseen_mailbox_messages(messages.clone(), Some("missing"));

        assert_eq!(unseen.len(), messages.len());
        assert_eq!(unseen[0].id, "msg_2");
        assert_eq!(unseen[1].id, "msg_1");
    }

    #[test]
    fn next_mailbox_cursor_after_refresh_advances_to_latest_processed_message() {
        let processed = vec![
            MoeMailMessageSummary {
                id: "msg_5".to_string(),
                subject: Some("latest".to_string()),
                received_at: Some("2026-03-16T05:00:00Z".to_string()),
            },
            MoeMailMessageSummary {
                id: "msg_4".to_string(),
                subject: Some("older".to_string()),
                received_at: Some("2026-03-16T04:00:00Z".to_string()),
            },
        ];

        let next = next_mailbox_cursor_after_refresh(Some("msg_3"), &processed);

        assert_eq!(next.as_deref(), Some("msg_5"));
    }

    #[test]
    fn next_mailbox_cursor_after_refresh_keeps_existing_cursor_when_nothing_was_processed() {
        let next = next_mailbox_cursor_after_refresh(Some("msg_3"), &[]);

        assert_eq!(next.as_deref(), Some("msg_3"));
    }

    #[test]
    fn merge_mailbox_code_prefers_fresher_refresh_value() {
        let stored = ParsedMailboxCode {
            value: "111111".to_string(),
            source: "subject".to_string(),
            updated_at: "2026-03-16T00:00:00Z".to_string(),
        };
        let fresh = ParsedMailboxCode {
            value: "222222".to_string(),
            source: "subject".to_string(),
            updated_at: "2026-03-16T00:01:00Z".to_string(),
        };

        let merged = merge_mailbox_code(Some(fresh), Some(stored)).expect("merged code");

        assert_eq!(merged.value, "222222");
        assert_eq!(merged.updated_at, "2026-03-16T00:01:00Z");
    }

    #[test]
    fn merge_mailbox_invite_keeps_newer_stored_value_when_refresh_is_older() {
        let stored = ParsedMailboxInvite {
            subject: "New invite".to_string(),
            copy_value: "https://example.com/new".to_string(),
            copy_label: "invite-link".to_string(),
            updated_at: "2026-03-16T00:05:00Z".to_string(),
        };
        let fresh = ParsedMailboxInvite {
            subject: "Old invite".to_string(),
            copy_value: "https://example.com/old".to_string(),
            copy_label: "invite-link".to_string(),
            updated_at: "2026-03-16T00:01:00Z".to_string(),
        };

        let merged = merge_mailbox_invite(Some(fresh), Some(stored)).expect("merged invite");

        assert_eq!(merged.subject, "New invite");
        assert_eq!(merged.copy_value, "https://example.com/new");
    }

    #[test]
    fn generate_mailbox_local_name_looks_like_human_or_org_style() {
        let local = generate_mailbox_local_name().expect("mailbox local part");
        assert!(local.len() >= 10);
        assert!(
            local
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '.' || ch == '-')
        );
        assert!(local.chars().any(|ch| ch.is_ascii_digit()));
        assert!(!local.starts_with('-'));
        assert!(!local.ends_with('-'));
    }

    #[test]
    fn random_base36_uses_letters_and_digits() {
        let token = random_base36(24).expect("base36 token");
        assert_eq!(token.len(), 24);
        assert!(
            token
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        );
        assert!(token.chars().any(|ch| ch.is_ascii_lowercase()));
        assert!(token.chars().any(|ch| ch.is_ascii_digit()));
    }

    #[test]
    fn build_window_usage_range_aligns_to_current_reset_window() {
        let now = parse_rfc3339_utc("2026-03-30T12:30:00Z").expect("fixed now");
        let range = build_window_usage_range(now, 300, Some("2026-03-30T14:00:00Z"))
            .expect("aligned range");

        assert_eq!(
            range.start_at,
            parse_rfc3339_utc("2026-03-30T09:00:00Z").expect("expected start")
        );
        assert_eq!(range.end_at, now);
    }

    #[test]
    fn build_window_usage_range_reuses_stale_reset_window_bounds() {
        let now = parse_rfc3339_utc("2026-03-30T12:30:00Z").expect("fixed now");
        let range = build_window_usage_range(now, 300, Some("2026-03-29T23:00:00Z"))
            .expect("historical range");

        assert_eq!(
            range.start_at,
            parse_rfc3339_utc("2026-03-29T18:00:00Z").expect("expected historical start")
        );
        assert_eq!(
            range.end_at,
            parse_rfc3339_utc("2026-03-29T23:00:00Z").expect("expected historical end")
        );
    }

    #[tokio::test]
    async fn enrich_window_actual_usage_for_summaries_counts_live_window_rows() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        ensure_window_actual_usage_test_tables(&state.pool).await;

        let account_id = insert_oauth_account(&state.pool, "Live Usage OAuth").await;
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            &format_utc_iso(Utc::now()),
            Some(27.0),
            Some(61.0),
        )
        .await;

        let primary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(45));
        let secondary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
        let failed_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(10));

        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &primary_row_at,
            Some(2400),
            Some(1200),
            Some(600),
            Some(4200),
            Some(0.042),
        )
        .await;
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &secondary_row_at,
            Some(1000),
            Some(500),
            Some(250),
            Some(1750),
            Some(0.0175),
        )
        .await;
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &failed_row_at,
            None,
            None,
            None,
            None,
            None,
        )
        .await;
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id + 999,
            &primary_row_at,
            Some(999),
            Some(999),
            Some(999),
            Some(2997),
            Some(0.2997),
        )
        .await;

        let mut summaries = load_upstream_account_summaries(&state.pool)
            .await
            .expect("load upstream account summaries");
        enrich_window_actual_usage_for_summaries(state.as_ref(), &mut summaries)
            .await
            .expect("enrich actual usage");

        let summary = summaries
            .into_iter()
            .find(|item| item.id == account_id)
            .expect("summary exists");
        let primary_usage = summary
            .primary_window
            .and_then(|window| window.actual_usage)
            .expect("primary actual usage");
        let secondary_usage = summary
            .secondary_window
            .and_then(|window| window.actual_usage)
            .expect("secondary actual usage");

        assert_eq!(primary_usage.request_count, 2);
        assert_eq!(primary_usage.total_tokens, 4200);
        assert_eq!(primary_usage.input_tokens, 2400);
        assert_eq!(primary_usage.output_tokens, 1200);
        assert_eq!(primary_usage.cache_input_tokens, 600);
        assert_cost_close(primary_usage.total_cost, 0.042);

        assert_eq!(secondary_usage.request_count, 3);
        assert_eq!(secondary_usage.total_tokens, 5950);
        assert_eq!(secondary_usage.input_tokens, 3400);
        assert_eq!(secondary_usage.output_tokens, 1700);
        assert_eq!(secondary_usage.cache_input_tokens, 850);
        assert_cost_close(secondary_usage.total_cost, 0.0595);
    }

    #[tokio::test]
    async fn enrich_window_actual_usage_for_summaries_uses_matching_stale_reset_window() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        ensure_window_actual_usage_test_tables(&state.pool).await;

        let account_id = 402_i64;
        let reset_at = Utc::now() - ChronoDuration::hours(10);
        let mut summary = test_summary_with_statuses(
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
        );
        summary.id = account_id;
        summary.primary_window = Some(RateWindowSnapshot {
            used_percent: 100.0,
            used_text: "100% used".to_string(),
            limit_text: "5h window".to_string(),
            resets_at: Some(format_utc_iso(reset_at)),
            window_duration_mins: 300,
            actual_usage: None,
        });

        let inside_window_at = shanghai_local_iso(reset_at - ChronoDuration::hours(1));
        let before_window_at = shanghai_local_iso(reset_at - ChronoDuration::hours(6));
        let after_window_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(30));

        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &inside_window_at,
            Some(1800),
            Some(900),
            Some(450),
            Some(3150),
            Some(0.0315),
        )
        .await;
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &before_window_at,
            Some(3000),
            Some(1200),
            Some(600),
            Some(4800),
            Some(0.048),
        )
        .await;
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &after_window_at,
            Some(500),
            Some(250),
            Some(100),
            Some(850),
            Some(0.0085),
        )
        .await;

        let mut items = vec![summary];
        enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
            .await
            .expect("enrich stale window actual usage");

        let usage = items[0]
            .primary_window
            .as_ref()
            .and_then(|window| window.actual_usage)
            .expect("stale primary actual usage");
        assert_eq!(usage.request_count, 1);
        assert_eq!(usage.total_tokens, 3150);
        assert_eq!(usage.input_tokens, 1800);
        assert_eq!(usage.output_tokens, 900);
        assert_eq!(usage.cache_input_tokens, 450);
        assert_cost_close(usage.total_cost, 0.0315);
    }

    #[tokio::test]
    async fn enrich_window_actual_usage_for_summaries_reads_archived_rows_past_retention_cutoff() {
        let mut config =
            usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
        config.invocation_max_days = 1;
        config.archive_dir = PathBuf::from(format!(
            "target/archive-tests/window-actual-usage-{}",
            random_base36(8).expect("archive suffix")
        ));
        let state = test_app_state_with_config_and_parallelism(
            config,
            DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
        )
        .await;
        ensure_window_actual_usage_test_tables(&state.pool).await;

        let account_id = 401_i64;
        let mut summary = test_summary_with_statuses(
            UPSTREAM_ACCOUNT_WORK_STATUS_IDLE,
            UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
        );
        summary.id = account_id;
        summary.primary_window = Some(RateWindowSnapshot {
            used_percent: 12.0,
            used_text: "12% used".to_string(),
            limit_text: "3d rolling window".to_string(),
            resets_at: None,
            window_duration_mins: 60 * 24 * 3,
            actual_usage: None,
        });

        let live_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::hours(6));
        let archived_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(2));
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &live_row_at,
            Some(1800),
            Some(900),
            Some(300),
            Some(3000),
            Some(0.03),
        )
        .await;
        seed_window_actual_usage_archive_batch(
            &state.pool,
            &state.config.archive_dir,
            "window-actual-usage-archive",
            &[(
                account_id,
                archived_row_at,
                Some(1200),
                Some(600),
                Some(200),
                Some(2000),
                Some(0.02),
            )],
        )
        .await;

        let mut items = vec![summary];
        enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
            .await
            .expect("enrich actual usage with archive rows");

        let usage = items[0]
            .primary_window
            .as_ref()
            .and_then(|window| window.actual_usage)
            .expect("primary actual usage");
        assert_eq!(usage.request_count, 2);
        assert_eq!(usage.total_tokens, 5000);
        assert_eq!(usage.input_tokens, 3000);
        assert_eq!(usage.output_tokens, 1500);
        assert_eq!(usage.cache_input_tokens, 500);
        assert_cost_close(usage.total_cost, 0.05);
    }

    #[tokio::test]
    async fn load_upstream_account_detail_with_actual_usage_serializes_actual_usage_camel_case() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        ensure_window_actual_usage_test_tables(&state.pool).await;

        let account_id = insert_oauth_account(&state.pool, "Detail Usage OAuth").await;
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            &format_utc_iso(Utc::now()),
            Some(33.0),
            Some(55.0),
        )
        .await;

        let primary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::minutes(25));
        let secondary_row_at = shanghai_local_iso(Utc::now() - ChronoDuration::days(1));
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &primary_row_at,
            Some(2100),
            Some(900),
            Some(300),
            Some(3300),
            Some(0.033),
        )
        .await;
        insert_window_actual_usage_invocation(
            &state.pool,
            account_id,
            &secondary_row_at,
            Some(700),
            Some(200),
            Some(100),
            Some(1000),
            Some(0.01),
        )
        .await;

        let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
            .await
            .expect("load detail with actual usage")
            .expect("detail exists");
        let primary_usage = detail
            .summary
            .primary_window
            .as_ref()
            .and_then(|window| window.actual_usage)
            .expect("primary actual usage");
        let secondary_usage = detail
            .summary
            .secondary_window
            .as_ref()
            .and_then(|window| window.actual_usage)
            .expect("secondary actual usage");

        assert_eq!(primary_usage.request_count, 1);
        assert_eq!(primary_usage.total_tokens, 3300);
        assert_cost_close(primary_usage.total_cost, 0.033);

        assert_eq!(secondary_usage.request_count, 2);
        assert_eq!(secondary_usage.total_tokens, 4300);
        assert_cost_close(secondary_usage.total_cost, 0.043);

        let payload = serde_json::to_value(&detail).expect("serialize detail payload");
        assert_eq!(payload["primaryWindow"]["actualUsage"]["requestCount"], 1);
        assert_eq!(payload["primaryWindow"]["actualUsage"]["totalTokens"], 3300);
        assert_eq!(payload["primaryWindow"]["actualUsage"]["inputTokens"], 2100);
        assert_eq!(payload["primaryWindow"]["actualUsage"]["outputTokens"], 900);
        assert_eq!(
            payload["primaryWindow"]["actualUsage"]["cacheInputTokens"],
            300
        );
        assert_eq!(payload["secondaryWindow"]["actualUsage"]["requestCount"], 2);
        assert_eq!(
            payload["secondaryWindow"]["actualUsage"]["totalTokens"],
            4300
        );
    }
}
