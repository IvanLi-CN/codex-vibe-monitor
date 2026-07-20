use super::*;

use futures_util::FutureExt;
use std::{any::Any, collections::BTreeMap, panic::AssertUnwindSafe};
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
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN";

pub(crate) const LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL";
pub(crate) const LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY";
pub(crate) const LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN: &str =
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
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM: usize = 4;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS: u64 = 60 * 60;
pub(crate) const DEFAULT_MANUAL_OAUTH_CALLBACK_PORT: u16 = 1455;
pub(crate) const MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: u64 = 60;
pub(crate) const UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS: u64 = 60;
pub(crate) const UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS: i64 = 6 * 60 * 60;
pub(crate) const OAUTH_MAILBOX_SOURCE_GENERATED: &str = "generated";
pub(crate) const OAUTH_MAILBOX_SOURCE_ATTACHED: &str = "attached";

pub(crate) const UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX: &str = "oauth_codex";
pub(crate) const UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX: &str = "api_key_codex";
pub(crate) const UPSTREAM_ACCOUNT_PROVIDER_CODEX: &str = "codex";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME: &str = "未分组";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_ACTIVE: &str = "active";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_SYNCING: &str = "syncing";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH: &str = "needs_reauth";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_ERROR: &str = "error";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_DISABLED: &str = "disabled";
pub(crate) const UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED: &str = "enabled";
pub(crate) const UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED: &str = "disabled";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_WORKING: &str = "working";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED: &str = "degraded";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_IDLE: &str = "idle";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED: &str = "rate_limited";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE: &str = "unavailable";
pub(crate) const UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL: &str = "normal";
pub(crate) const UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE: &str =
    "upstream_unavailable";
pub(crate) const UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED: &str = "upstream_rejected";
pub(crate) const UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER: &str = "error_other";
pub(crate) const UPSTREAM_ACCOUNT_SYNC_STATE_IDLE: &str = "idle";
pub(crate) const UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING: &str = "syncing";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED: &str = "route_recovered";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED: &str = "route_cooldown_started";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE: &str = "route_retryable_failure";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE: &str = "route_hard_unavailable";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED: &str =
    "status_change_suppressed";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED: &str = "sync_succeeded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_DEFERRED: &str = "sync_deferred";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE: &str = "sync_hard_unavailable";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED: &str = "sync_recovery_blocked";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED: &str = "sync_failed";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED: &str = "account_updated";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL: &str = "call";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL: &str = "sync_manual";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE: &str = "sync_maintenance";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_POST_CREATE: &str = "sync_post_create";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT: &str = "oauth_import";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_ACCOUNT_UPDATE: &str = "account_update";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_OK: &str = "sync_ok";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED: &str = "account_updated";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR: &str = "sync_error";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED: &str = "egress_throttled";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED: &str =
    "usage_snapshot_exhausted";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED: &str =
    "quota_still_exhausted";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED: &str =
    "recovery_unconfirmed_manual_required";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401: &str = "upstream_http_401";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402: &str = "upstream_http_402";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403: &str = "upstream_http_403";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT: &str =
    "upstream_http_429_rate_limit";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED: &str =
    "upstream_http_429_quota_exhausted";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE: &str = "transport_failure";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED: &str =
    "upstream_server_overloaded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED: &str = "reauth_required";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX: &str = "upstream_http_5xx";
pub(crate) const LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED: &str =
    "upstream_rejected";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED: &str =
    "group_node_shunt_unassigned";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE: &str =
    "分组节点分流策略控制，未排节点";
pub(crate) const UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED: &str = "assigned";
pub(crate) const UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_PENDING: &str = "pending";
pub(crate) const UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED: &str = "unconfigured";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE: &str = "enable";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE: &str = "disable";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_DELETE: &str = "delete";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP: &str = "set_group";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS: &str = "add_tags";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS: &str = "remove_tags";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING: &str = "pending";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED: &str = "succeeded";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED: &str = "failed";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED: &str = "skipped";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING: &str = "running";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED: &str = "completed";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED: &str = "failed";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED: &str = "cancelled";
pub(crate) const LOGIN_SESSION_STATUS_PENDING: &str = "pending";
pub(crate) const LOGIN_SESSION_STATUS_COMPLETED: &str = "completed";
pub(crate) const LOGIN_SESSION_STATUS_FAILED: &str = "failed";
pub(crate) const LOGIN_SESSION_STATUS_EXPIRED: &str = "expired";
pub(crate) const LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION: &str =
    "needs_identity_confirmation";
pub(crate) const LOGIN_SESSION_BASE_UPDATED_AT_HEADER: &str =
    "x-codex-login-session-base-updated-at";
pub(crate) const IMPORT_VALIDATION_STATUS_OK: &str = "ok";
pub(crate) const IMPORT_VALIDATION_STATUS_OK_EXHAUSTED: &str = "ok_exhausted";
pub(crate) const IMPORT_VALIDATION_STATUS_INVALID: &str = "invalid";
pub(crate) const IMPORT_VALIDATION_STATUS_ERROR: &str = "error";
pub(crate) const IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT: &str = "duplicate_in_input";
pub(crate) const IMPORT_RESULT_STATUS_CREATED: &str = "created";
pub(crate) const IMPORT_RESULT_STATUS_UPDATED_EXISTING: &str = "updated_existing";
pub(crate) const IMPORT_RESULT_STATUS_FAILED: &str = "failed";
pub(crate) const DEFAULT_OAUTH_SCOPE: &str = "openid profile email offline_access";
pub(crate) const DEFAULT_OAUTH_AUDIENCE: &str = "https://api.openai.com/v1";
pub(crate) const DEFAULT_OAUTH_PROMPT: &str = "login";
pub(crate) const OAUTH_ORIGINATOR: &str = "Codex Desktop";
pub(crate) const DEFAULT_USAGE_LIMIT_ID: &str = "codex";
pub(crate) const DEFAULT_API_KEY_LIMIT_UNIT: &str = "requests";
pub(crate) const POOL_SETTINGS_SINGLETON_ID: i64 = 1;
pub(crate) const DEFAULT_STICKY_KEY_LIMIT: i64 = 50;
pub(crate) const STICKY_KEY_ACTIVITY_MODE_LIMIT: i64 = 50;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE: usize = 20;
pub(crate) const UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS: [usize; 3] = [20, 50, 100];
pub(crate) const POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES: i64 = 5;
pub(crate) const POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD: i64 = 5;
pub(crate) const POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS: i64 = 30;
pub(crate) const POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS: i64 = 60;
pub(crate) const STATUS_CHANGE_REASON_CODES: [&str; 11] = [
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403,
    UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
    UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
    UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
    UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
];
pub(crate) const COMPACT_SUPPORT_STATUS_UNKNOWN: &str = "unknown";
pub(crate) const COMPACT_SUPPORT_STATUS_SUPPORTED: &str = "supported";
pub(crate) const COMPACT_SUPPORT_STATUS_UNSUPPORTED: &str = "unsupported";
pub(crate) const USAGE_PATH_STYLE_CHATGPT: &str = "/wham/usage";
pub(crate) const USAGE_PATH_STYLE_CODEX_API: &str = "/api/codex/usage";
pub(crate) const UPSTREAM_USAGE_BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

#[derive(Debug)]
pub(crate) struct UpstreamAccountsRuntime {
    pub(crate) crypto_key: Option<[u8; 32]>,
    pub(crate) account_ops: AccountOpCoordinator,
    pub(crate) validation_jobs: Arc<Mutex<HashMap<String, Arc<ImportedOauthValidationJob>>>>,
    pub(crate) bulk_sync_jobs: Arc<Mutex<HashMap<String, Arc<BulkUpstreamAccountSyncJob>>>>,
    pub(crate) bulk_sync_creation: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountCommand {
    UpdateAccount,
    ExternalOauthUpsert,
    DeleteAccount,
    ManualSync,
    MaintenanceSync,
    PersistOauthCallback,
    PersistImportedOauth,
    ConfirmOauthIdentityOverwrite,
    PostCreateSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncCause {
    Manual,
    Maintenance,
    PostCreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SyncSuccessRouteState {
    PreserveFailureState,
    ClearFailureState,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaintenanceDispatchOutcome {
    Executed,
    Skipped,
    Deduped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaintenanceQueueOutcome {
    Queued,
    Deduped,
}

pub(crate) struct MaintenancePendingGuard {
    pub(crate) flag: Arc<AtomicBool>,
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
pub(crate) struct AccountActorHandle {
    pub(crate) serial: Arc<tokio::sync::Mutex<()>>,
    pub(crate) maintenance_pending: Arc<AtomicBool>,
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
pub(crate) struct AccountOpCoordinator {
    pub(crate) actors: Arc<std::sync::Mutex<HashMap<i64, AccountActorHandle>>>,
    pub(crate) maintenance_slots: Arc<tokio::sync::Semaphore>,
    pub(crate) maintenance_handles: Arc<std::sync::Mutex<Vec<JoinHandle<()>>>>,
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
pub(crate) enum AccountCommandDispatchError<E> {
    Command(E),
    ActorUnavailable(AccountCommand),
}

#[derive(Debug)]
pub(crate) enum AccountSubmitOutcome<T> {
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

    pub(crate) async fn insert_validation_job(
        &self,
        job_id: String,
        job: Arc<ImportedOauthValidationJob>,
    ) {
        self.validation_jobs.lock().await.insert(job_id, job);
    }

    pub(crate) async fn get_validation_job(
        &self,
        job_id: &str,
    ) -> Option<Arc<ImportedOauthValidationJob>> {
        self.validation_jobs.lock().await.get(job_id).cloned()
    }

    pub(crate) async fn remove_validation_job(
        &self,
        job_id: &str,
    ) -> Option<Arc<ImportedOauthValidationJob>> {
        self.validation_jobs.lock().await.remove(job_id)
    }

    pub(crate) async fn insert_bulk_sync_job(
        &self,
        job_id: String,
        job: Arc<BulkUpstreamAccountSyncJob>,
    ) {
        self.bulk_sync_jobs.lock().await.insert(job_id, job);
    }

    pub(crate) async fn get_bulk_sync_job(
        &self,
        job_id: &str,
    ) -> Option<Arc<BulkUpstreamAccountSyncJob>> {
        self.bulk_sync_jobs.lock().await.get(job_id).cloned()
    }

    pub(crate) async fn get_running_bulk_sync_job(
        &self,
    ) -> Option<(String, Arc<BulkUpstreamAccountSyncJob>)> {
        let jobs = self.bulk_sync_jobs.lock().await;
        for (job_id, job) in jobs.iter() {
            if job.terminal_event.lock().await.is_none() {
                return Some((job_id.clone(), job.clone()));
            }
        }
        None
    }

    pub(crate) async fn remove_bulk_sync_job(
        &self,
        job_id: &str,
    ) -> Option<Arc<BulkUpstreamAccountSyncJob>> {
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
    pub(crate) fn actor_count(&self) -> usize {
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

    pub(crate) async fn submit_command<R, E, F, Fut>(
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

    pub(crate) async fn run_update_account(
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

    pub(crate) async fn run_external_oauth_upsert(
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

    pub(crate) async fn run_delete_account(
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

    pub(crate) async fn run_manual_sync(
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

    pub(crate) async fn run_post_create_sync(
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
    pub(crate) async fn run_maintenance_sync(
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

    pub(crate) async fn run_persist_oauth_callback(
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

    pub(crate) async fn run_persist_imported_oauth(
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

    pub(crate) async fn run_confirm_oauth_identity_overwrite(
        &self,
        state: Arc<AppState>,
        login_id: String,
    ) -> Result<i64, (StatusCode, String)> {
        let account_id = load_login_session_by_login_id(&state.pool, &login_id)
            .await
            .map_err(internal_error_tuple)?
            .and_then(|session| session.account_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
        self.submit_command(
            state,
            account_id,
            AccountCommand::ConfirmOauthIdentityOverwrite,
            false,
            move |state, _| async move {
                confirm_oauth_identity_overwrite_inner(state.as_ref(), &login_id).await
            },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::ConfirmOauthIdentityOverwrite)
    }

    pub(crate) fn dispatch_maintenance_sync(
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

pub(crate) fn describe_panic_payload(payload: &Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

pub(crate) fn map_account_dispatch_http(
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

pub(crate) fn map_account_dispatch_anyhow(
    err: AccountCommandDispatchError<anyhow::Error>,
) -> anyhow::Error {
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
    pub(crate) total: usize,
    pub(crate) oauth: usize,
    pub(crate) api_key: usize,
    pub(crate) attention: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountListResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) items: Vec<UpstreamAccountSummary>,
    pub(crate) total: usize,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
    pub(crate) metrics: UpstreamAccountListMetrics,
    pub(crate) groups: Vec<UpstreamAccountGroupSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) forward_proxy_nodes: Vec<ForwardProxyBindingNodeResponse>,
    pub(crate) has_ungrouped_accounts: bool,
    pub(crate) routing: PoolRoutingSettingsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActionEventListResponse {
    pub(crate) items: Vec<UpstreamAccountActionEvent>,
    pub(crate) total: usize,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountAttemptListResponse {
    pub(crate) items: Vec<ApiPoolUpstreamRequestAttempt>,
    pub(crate) total: usize,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocateUpstreamAccountAttemptQuery {
    pub(crate) attempt_id: String,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountAttemptsQuery {
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountWindowUsageRequest {
    #[serde(default)]
    pub(crate) account_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountWindowUsageItem {
    pub(crate) account_id: i64,
    pub(crate) primary_actual_usage: Option<RateWindowActualUsage>,
    pub(crate) secondary_actual_usage: Option<RateWindowActualUsage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountWindowUsageResponse {
    pub(crate) items: Vec<UpstreamAccountWindowUsageItem>,
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
pub(crate) struct ListUpstreamAccountActionEventsQuery {
    pub(crate) account: Option<String>,
    pub(crate) group: Option<String>,
    pub(crate) proxy_key: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountsQuery {
    #[serde(default)]
    pub(crate) group_exact: Vec<String>,
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
    pub(crate) include_all: Option<bool>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountsBaseQuery {
    pub(crate) group_search: Option<String>,
    pub(crate) group_ungrouped: Option<bool>,
    pub(crate) status: Option<String>,
    pub(crate) page: Option<usize>,
    pub(crate) page_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DuplicateReason {
    SharedChatgptAccountId,
    SharedChatgptUserId,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub(crate) enum TagPriorityTier {
    NoNew,
    Fallback,
    #[default]
    Normal,
    Primary,
}

impl TagPriorityTier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoNew => "no_new",
            Self::Fallback => "fallback",
            Self::Normal => "normal",
            Self::Primary => "primary",
        }
    }

    pub(crate) fn routing_rank(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Normal => 1,
            Self::Fallback => 2,
            Self::NoNew => 3,
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

    pub(crate) fn merge_rank(self) -> u8 {
        match self {
            Self::ForceRemove => 0,
            Self::ForceAdd => 1,
            Self::FillMissing => 2,
            Self::KeepOriginal => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ImageToolRewriteMode {
    ForceRemove,
    #[default]
    KeepOriginal,
    FillMissing,
    ForceAdd,
}

impl ImageToolRewriteMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ForceRemove => "force_remove",
            Self::KeepOriginal => "keep_original",
            Self::FillMissing => "fill_missing",
            Self::ForceAdd => "force_add",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "force_remove" => Self::ForceRemove,
            "fill_missing" => Self::FillMissing,
            "force_add" => Self::ForceAdd,
            _ => Self::KeepOriginal,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequestCompressionAlgorithm {
    Follow,
    #[default]
    Identity,
    Gzip,
    Deflate,
    Zstd,
}

impl RequestCompressionAlgorithm {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Follow => "follow",
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
            Self::Zstd => "zstd",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "follow" => Self::Follow,
            "gzip" => Self::Gzip,
            "deflate" => Self::Deflate,
            "zstd" => Self::Zstd,
            _ => Self::Identity,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RequestCompressionLevelPreset {
    Fast,
    #[default]
    Balanced,
    Best,
}

impl RequestCompressionLevelPreset {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Best => "best",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "fast" => Self::Fast,
            "best" => Self::Best,
            _ => Self::Balanced,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CapabilitySupport {
    Supported,
    Unsupported,
    #[default]
    Unknown,
}

impl CapabilitySupport {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "supported" => Self::Supported,
            "unsupported" => Self::Unsupported,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamCapabilityState {
    pub(crate) observed: CapabilitySupport,
    #[serde(rename = "override")]
    pub(crate) override_value: Option<CapabilitySupport>,
    pub(crate) effective: CapabilitySupport,
    pub(crate) observed_at: Option<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ImageIntent {
    Yes,
    DirectImage,
    No,
    #[default]
    Unknown,
}

impl ImageIntent {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::DirectImage => "direct_image",
            Self::No => "no",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> Self {
        match value.trim() {
            "yes" => Self::Yes,
            "direct_image" => Self::DirectImage,
            "no" => Self::No,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RequestCapabilityRequirements {
    pub(crate) response_endpoint: bool,
    pub(crate) chat_completions_endpoint: bool,
    pub(crate) image_endpoint: bool,
    pub(crate) response_image_tool: bool,
}

impl RequestCapabilityRequirements {
    pub(crate) fn from_endpoint_and_image_intent(
        endpoint: &str,
        image_intent: ImageIntent,
    ) -> Self {
        match endpoint {
            "/v1/responses" | "/v1/responses/compact" => Self::response_family(image_intent),
            "/v1/chat/completions" => Self::chat_completions(),
            "/v1/images/generations" | "/v1/images/edits" => Self::direct_image_endpoint(),
            _ => Self::default(),
        }
    }

    pub(crate) fn direct_image_endpoint() -> Self {
        Self {
            response_endpoint: false,
            chat_completions_endpoint: false,
            image_endpoint: true,
            response_image_tool: false,
        }
    }

    pub(crate) fn chat_completions() -> Self {
        Self {
            response_endpoint: false,
            chat_completions_endpoint: true,
            image_endpoint: false,
            response_image_tool: false,
        }
    }

    pub(crate) fn response_family(image_intent: ImageIntent) -> Self {
        Self {
            response_endpoint: true,
            chat_completions_endpoint: false,
            image_endpoint: false,
            response_image_tool: matches!(image_intent, ImageIntent::Yes),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DuplicateInfo {
    pub(crate) peer_account_ids: Vec<i64>,
    pub(crate) reasons: Vec<DuplicateReason>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountTagSummary {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) routing_rule: TagRoutingRule,
    pub(crate) system_key: Option<String>,
    pub(crate) protected: bool,
}

pub(crate) type StatusChangeReasonSettings = BTreeMap<String, bool>;
pub(crate) type StatusChangeReasonFieldSources = BTreeMap<String, String>;

pub(crate) fn canonical_status_change_reason_code(reason_code: &str) -> Option<&'static str> {
    match reason_code.trim() {
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401 => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402
        | LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403 => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
        }
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX => {
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX)
        }
        _ => None,
    }
}

pub(crate) fn default_status_change_reasons() -> StatusChangeReasonSettings {
    STATUS_CHANGE_REASON_CODES
        .into_iter()
        .map(|reason_code| (reason_code.to_string(), true))
        .collect()
}

pub(crate) fn default_status_change_reason_field_sources(
    source: &str,
) -> StatusChangeReasonFieldSources {
    STATUS_CHANGE_REASON_CODES
        .into_iter()
        .map(|reason_code| (reason_code.to_string(), source.to_string()))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectiveRoutingRuleFieldSources {
    pub(crate) allow_cut_out: String,
    pub(crate) allow_cut_in: String,
    pub(crate) priority_tier: String,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) image_tool_rewrite_mode: String,
    pub(crate) request_compression_algorithm: String,
    pub(crate) concurrency_limit: String,
    pub(crate) upstream_429_retry: String,
    pub(crate) available_models: String,
    pub(crate) system_denied_models: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingTimeoutFieldSources {
    pub(crate) responses_first_byte_timeout_secs: String,
    pub(crate) compact_first_byte_timeout_secs: String,
    pub(crate) image_first_byte_timeout_secs: String,
    pub(crate) responses_stream_timeout_secs: String,
    pub(crate) compact_stream_timeout_secs: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RoutingTimeoutSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) responses_first_byte_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) compact_first_byte_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) image_first_byte_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) responses_stream_timeout_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) compact_stream_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EffectiveRoutingRule {
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: TagPriorityTier,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) image_tool_rewrite_mode: ImageToolRewriteMode,
    pub(crate) request_compression_algorithm: RequestCompressionAlgorithm,
    pub(crate) concurrency_limit: i64,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) available_models: Vec<String>,
    #[serde(skip)]
    pub(crate) available_models_defined: bool,
    pub(crate) status_change_reasons: StatusChangeReasonSettings,
    pub(crate) status_change_reason_field_sources: StatusChangeReasonFieldSources,
    pub(crate) system_denied_models: Vec<String>,
    pub(crate) source_tag_ids: Vec<i64>,
    pub(crate) source_tag_names: Vec<String>,
    pub(crate) field_sources: EffectiveRoutingRuleFieldSources,
    pub(crate) timeouts: RoutingTimeoutSettings,
    pub(crate) timeout_field_sources: RoutingTimeoutFieldSources,
}

impl EffectiveRoutingRule {
    pub(crate) fn allow_cut_out(&self) -> bool {
        self.allow_cut_out
    }

    pub(crate) fn allow_cut_out_source(&self) -> &str {
        &self.field_sources.allow_cut_out
    }

    pub(crate) fn fast_mode_rewrite_mode_source(&self) -> &str {
        &self.field_sources.fast_mode_rewrite_mode
    }

    pub(crate) fn image_tool_rewrite_mode_source(&self) -> &str {
        &self.field_sources.image_tool_rewrite_mode
    }

    pub(crate) fn request_compression_algorithm_source(&self) -> &str {
        &self.field_sources.request_compression_algorithm
    }

    pub(crate) fn available_models(&self) -> Option<&[String]> {
        self.available_models_defined
            .then_some(self.available_models.as_slice())
    }

    pub(crate) fn available_models_source(&self) -> &str {
        &self.field_sources.available_models
    }

    pub(crate) fn status_change_reason_enabled(&self, reason_code: &str) -> bool {
        canonical_status_change_reason_code(reason_code)
            .and_then(|reason_code| self.status_change_reasons.get(reason_code))
            .copied()
            .unwrap_or(true)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ConversationRoutingOverride {
    pub(crate) allow_switch_upstream: Option<bool>,
    pub(crate) fast_mode_rewrite_mode: Option<TagFastModeRewriteMode>,
    pub(crate) image_tool_rewrite_mode: Option<ImageToolRewriteMode>,
    pub(crate) available_models: Option<Vec<String>>,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_keys: Vec<String>,
    pub(crate) forward_proxy_scope_key: String,
}

impl ConversationRoutingOverride {
    pub(crate) fn has_policy_override(&self) -> bool {
        self.allow_switch_upstream.is_some()
            || self.fast_mode_rewrite_mode.is_some()
            || self.image_tool_rewrite_mode.is_some()
            || self.available_models.is_some()
            || self.forward_proxy_key.is_some()
            || !self.forward_proxy_keys.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagRoutingRule {
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: TagPriorityTier,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) concurrency_limit: i64,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) available_models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupAccountRoutingRule {
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: TagPriorityTier,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) image_tool_rewrite_mode: ImageToolRewriteMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_compression_algorithm: Option<RequestCompressionAlgorithm>,
    pub(crate) concurrency_limit: i64,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) available_models: Vec<String>,
    pub(crate) available_models_defined: bool,
    pub(crate) status_change_reasons: StatusChangeReasonSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timeouts: Option<RoutingTimeoutSettings>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagSummary {
    pub(crate) id: i64,
    pub(crate) name: String,
    pub(crate) routing_rule: TagRoutingRule,
    pub(crate) account_count: i64,
    pub(crate) group_count: i64,
    pub(crate) updated_at: String,
    pub(crate) system_key: Option<String>,
    pub(crate) protected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagDetail {
    #[serde(flatten)]
    pub(crate) summary: TagSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TagListResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) items: Vec<TagSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountGroupSummary {
    pub(crate) group_name: String,
    pub(crate) account_count: i64,
    pub(crate) note: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) node_shunt_enabled: bool,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) concurrency_limit: i64,
    pub(crate) routing_rule: GroupAccountRoutingRule,
    pub(crate) effective_timeouts: RoutingTimeoutSettings,
    pub(crate) timeout_field_sources: RoutingTimeoutFieldSources,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UpstreamAccountGroupMetadata {
    pub(crate) note: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) node_shunt_enabled: bool,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) concurrency_limit: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestedGroupMetadataChanges {
    pub(crate) note: Option<String>,
    pub(crate) note_was_requested: bool,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) bound_proxy_keys_was_requested: bool,
    pub(crate) concurrency_limit: i64,
    pub(crate) concurrency_limit_was_requested: bool,
    pub(crate) node_shunt_enabled: bool,
    pub(crate) node_shunt_enabled_was_requested: bool,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) single_account_rotation_enabled_was_requested: bool,
}

impl RequestedGroupMetadataChanges {
    pub(crate) fn was_requested(&self) -> bool {
        self.note_was_requested
            || self.bound_proxy_keys_was_requested
            || self.concurrency_limit_was_requested
            || self.node_shunt_enabled_was_requested
            || self.single_account_rotation_enabled_was_requested
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
    pub(crate) id: i64,
    pub(crate) kind: String,
    pub(crate) provider: String,
    pub(crate) display_name: String,
    pub(crate) group_name: Option<String>,
    pub(crate) is_mother: bool,
    pub(crate) status: String,
    pub(crate) display_status: String,
    pub(crate) enabled: bool,
    pub(crate) work_status: String,
    pub(crate) enable_status: String,
    pub(crate) health_status: String,
    pub(crate) sync_state: String,
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) plan_type: Option<String>,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) has_refresh_token: bool,
    pub(crate) last_synced_at: Option<String>,
    pub(crate) last_successful_sync_at: Option<String>,
    pub(crate) last_activity_at: Option<String>,
    pub(crate) active_conversation_count: i64,
    pub(crate) last_error: Option<String>,
    pub(crate) last_error_at: Option<String>,
    pub(crate) last_action: Option<String>,
    pub(crate) last_action_source: Option<String>,
    pub(crate) last_action_reason_code: Option<String>,
    pub(crate) last_action_reason_message: Option<String>,
    pub(crate) last_action_http_status: Option<u16>,
    pub(crate) last_action_invoke_id: Option<String>,
    pub(crate) last_action_at: Option<String>,
    pub(crate) cooldown_until: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) current_forward_proxy_key: Option<String>,
    pub(crate) current_forward_proxy_display_name: Option<String>,
    pub(crate) current_forward_proxy_state: String,
    pub(crate) routing_block_reason_code: Option<String>,
    pub(crate) routing_block_reason_message: Option<String>,
    pub(crate) token_expires_at: Option<String>,
    pub(crate) primary_window: Option<RateWindowSnapshot>,
    pub(crate) secondary_window: Option<RateWindowSnapshot>,
    pub(crate) credits: Option<CreditsSnapshot>,
    pub(crate) local_limits: Option<LocalLimitSnapshot>,
    pub(crate) compact_support: CompactSupportState,
    pub(crate) duplicate_info: Option<DuplicateInfo>,
    pub(crate) tags: Vec<AccountTagSummary>,
    pub(crate) effective_routing_rule: EffectiveRoutingRule,
    pub(crate) response_endpoint_capability: UpstreamCapabilityState,
    pub(crate) chat_completions_capability: UpstreamCapabilityState,
    pub(crate) image_endpoint_capability: UpstreamCapabilityState,
    pub(crate) response_image_tool_capability: UpstreamCapabilityState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountDetail {
    #[serde(flatten)]
    pub(crate) summary: UpstreamAccountSummary,
    pub(crate) note: Option<String>,
    pub(crate) upstream_base_url: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) last_refreshed_at: Option<String>,
    pub(crate) history: Vec<UpstreamAccountHistoryPoint>,
    pub(crate) recent_actions: Vec<UpstreamAccountActionEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountActionEvent {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) action: String,
    pub(crate) source: String,
    pub(crate) account_display_name: Option<String>,
    pub(crate) account_group_name: Option<String>,
    pub(crate) forward_proxy_key: Option<String>,
    pub(crate) forward_proxy_display_name: Option<String>,
    pub(crate) forward_proxy_egress_ip: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) result_description: Option<String>,
    pub(crate) reason_code: Option<String>,
    pub(crate) reason_message: Option<String>,
    pub(crate) http_status: Option<u16>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) invoke_id: Option<String>,
    pub(crate) attempt_id: Option<String>,
    pub(crate) sticky_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompactSupportState {
    pub(crate) status: String,
    pub(crate) observed_at: Option<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingTimeoutSettingsResponse {
    pub(crate) responses_first_byte_timeout_secs: u64,
    pub(crate) compact_first_byte_timeout_secs: u64,
    pub(crate) image_first_byte_timeout_secs: u64,
    pub(crate) responses_stream_timeout_secs: u64,
    pub(crate) compact_stream_timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RoutingTimeoutOverridesResolved {
    pub(crate) responses_first_byte_timeout: Option<Duration>,
    pub(crate) compact_first_byte_timeout: Option<Duration>,
    pub(crate) image_first_byte_timeout: Option<Duration>,
    pub(crate) responses_stream_timeout: Option<Duration>,
    pub(crate) compact_stream_timeout: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolRoutingTimeoutSettingsResolved {
    pub(crate) default_first_byte_timeout: Duration,
    pub(crate) default_send_timeout: Duration,
    pub(crate) request_read_timeout: Duration,
    pub(crate) responses_first_byte_timeout: Duration,
    pub(crate) compact_first_byte_timeout: Duration,
    pub(crate) image_first_byte_timeout: Duration,
    pub(crate) responses_stream_timeout: Duration,
    pub(crate) compact_stream_timeout: Duration,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PoolRoutingRequestCompressionSettingsResolved {
    pub(crate) algorithm: RequestCompressionAlgorithm,
    pub(crate) level_preset: RequestCompressionLevelPreset,
}

impl PoolRoutingTimeoutSettingsResolved {
    pub(crate) fn with_overrides(
        self,
        overrides: RoutingTimeoutOverridesResolved,
    ) -> PoolRoutingTimeoutSettingsResolved {
        PoolRoutingTimeoutSettingsResolved {
            default_first_byte_timeout: self.default_first_byte_timeout,
            default_send_timeout: self.default_send_timeout,
            request_read_timeout: self.request_read_timeout,
            responses_first_byte_timeout: overrides
                .responses_first_byte_timeout
                .unwrap_or(self.responses_first_byte_timeout),
            compact_first_byte_timeout: overrides
                .compact_first_byte_timeout
                .unwrap_or(self.compact_first_byte_timeout),
            image_first_byte_timeout: overrides
                .image_first_byte_timeout
                .unwrap_or(self.image_first_byte_timeout),
            responses_stream_timeout: overrides
                .responses_stream_timeout
                .unwrap_or(self.responses_stream_timeout),
            compact_stream_timeout: overrides
                .compact_stream_timeout
                .unwrap_or(self.compact_stream_timeout),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSettingsResponse {
    pub(crate) writes_enabled: bool,
    pub(crate) api_key_configured: bool,
    pub(crate) masked_api_key: Option<String>,
    pub(crate) maintenance: PoolRoutingMaintenanceSettingsResponse,
    pub(crate) request_compression_algorithm: RequestCompressionAlgorithm,
    pub(crate) request_compression_level_preset: RequestCompressionLevelPreset,
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
    pub(crate) request_compression_algorithm: Option<String>,
    #[serde(default)]
    pub(crate) request_compression_level_preset: Option<String>,
    #[serde(default)]
    pub(crate) timeouts: Option<UpdatePoolRoutingTimeoutSettingsRequest>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateRoutingTimeoutSettingsRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) responses_first_byte_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) compact_first_byte_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) image_first_byte_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) responses_stream_timeout_secs: OptionalField<u64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) compact_stream_timeout_secs: OptionalField<u64>,
}

impl UpdateRoutingTimeoutSettingsRequest {
    pub(crate) fn is_empty(&self) -> bool {
        matches!(
            self.responses_first_byte_timeout_secs,
            OptionalField::Missing
        ) && matches!(self.compact_first_byte_timeout_secs, OptionalField::Missing)
            && matches!(self.image_first_byte_timeout_secs, OptionalField::Missing)
            && matches!(self.responses_stream_timeout_secs, OptionalField::Missing)
            && matches!(self.compact_stream_timeout_secs, OptionalField::Missing)
    }
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
    pub(crate) image_first_byte_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) responses_stream_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) compact_stream_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeysResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) selection_mode: AccountStickyKeySelectionMode,
    pub(crate) selected_limit: Option<i64>,
    pub(crate) selected_activity_hours: Option<i64>,
    pub(crate) implicit_filter: AccountStickyKeyImplicitFilter,
    pub(crate) conversations: Vec<AccountStickyKeyConversation>,
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
    pub(crate) fn selection_mode(self) -> AccountStickyKeySelectionMode {
        match self {
            Self::Count(_) => AccountStickyKeySelectionMode::Count,
            Self::ActivityWindow(_) => AccountStickyKeySelectionMode::ActivityWindow,
        }
    }

    pub(crate) fn selected_limit(self) -> Option<i64> {
        match self {
            Self::Count(limit) => Some(limit),
            Self::ActivityWindow(_) => None,
        }
    }

    pub(crate) fn selected_activity_hours(self) -> Option<i64> {
        match self {
            Self::Count(_) => None,
            Self::ActivityWindow(hours) => Some(hours),
        }
    }

    pub(crate) fn activity_window_hours(self) -> i64 {
        match self {
            Self::Count(_) => 24,
            Self::ActivityWindow(hours) => hours,
        }
    }

    pub(crate) fn display_limit(self) -> i64 {
        match self {
            Self::Count(limit) => limit,
            Self::ActivityWindow(_) => STICKY_KEY_ACTIVITY_MODE_LIMIT,
        }
    }

    pub(crate) fn implicit_filter(
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
pub(crate) struct AccountStickyKeyFilteredCounts {
    pub(crate) inactive_count: i64,
    pub(crate) capped_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyImplicitFilter {
    pub(crate) kind: Option<AccountStickyKeyImplicitFilterKind>,
    pub(crate) filtered_count: i64,
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
    pub(crate) sticky_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
    pub(crate) recent_invocations:
        Vec<crate::api::PromptCacheConversationInvocationPreviewResponse>,
    pub(crate) last24h_requests: Vec<AccountStickyKeyRequestPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountStickyKeyRequestPoint {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) is_success: bool,
    pub(crate) request_tokens: i64,
    pub(crate) cumulative_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountHistoryPoint {
    pub(crate) captured_at: String,
    pub(crate) primary_used_percent: Option<f64>,
    pub(crate) secondary_used_percent: Option<f64>,
    pub(crate) credits_balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowSnapshot {
    pub(crate) used_percent: f64,
    pub(crate) used_text: String,
    pub(crate) limit_text: String,
    pub(crate) resets_at: Option<String>,
    pub(crate) window_duration_mins: i64,
    pub(crate) actual_usage: Option<RateWindowActualUsage>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowActualUsage {
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditsSnapshot {
    pub(crate) has_credits: bool,
    pub(crate) unlimited: bool,
    pub(crate) balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalLimitSnapshot {
    pub(crate) primary_limit: Option<f64>,
    pub(crate) secondary_limit: Option<f64>,
    pub(crate) limit_unit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginSessionStatusResponse {
    pub(crate) login_id: String,
    pub(crate) status: String,
    pub(crate) auth_url: Option<String>,
    pub(crate) redirect_uri: Option<String>,
    pub(crate) expires_at: String,
    pub(crate) updated_at: String,
    pub(crate) account_id: Option<i64>,
    pub(crate) email: Option<String>,
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sync_applied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) identity_confirmation: Option<OauthIdentityConfirmationResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthIdentityConfirmationResponse {
    pub(crate) current: OauthIdentitySummaryResponse,
    pub(crate) incoming: OauthIdentitySummaryResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthIdentitySummaryResponse {
    pub(crate) account_id: Option<i64>,
    pub(crate) display_name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) plan_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxSessionResponse {
    pub(crate) email_address: String,
    pub(crate) supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxCodeSummary {
    pub(crate) value: String,
    pub(crate) source: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthInviteSummary {
    pub(crate) subject: String,
    pub(crate) copy_value: String,
    pub(crate) copy_label: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatus {
    pub(crate) session_id: String,
    pub(crate) email_address: String,
    pub(crate) expires_at: String,
    pub(crate) latest_code: Option<OauthMailboxCodeSummary>,
    pub(crate) invite: Option<OauthInviteSummary>,
    pub(crate) invited: bool,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatusBatchResponse {
    pub(crate) items: Vec<OauthMailboxStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthLoginSessionRequest {
    pub(crate) display_name: Option<String>,
    pub(crate) email: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) account_id: Option<i64>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
    pub(crate) is_mother: Option<bool>,
    pub(crate) mailbox_session_id: Option<String>,
    #[serde(alias = "generatedMailboxAddress")]
    pub(crate) mailbox_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompleteOauthLoginSessionRequest {
    pub(crate) callback_url: String,
    pub(crate) mailbox_session_id: Option<String>,
    #[serde(alias = "generatedMailboxAddress")]
    pub(crate) mailbox_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateOauthLoginSessionRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) display_name: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) email: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_name: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_bound_proxy_keys: OptionalField<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_node_shunt_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_single_account_rotation_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) group_note: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) concurrency_limit: OptionalField<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) tag_ids: OptionalField<Vec<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) is_mother: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) mailbox_session_id: OptionalField<String>,
    #[serde(
        default,
        alias = "generatedMailboxAddress",
        deserialize_with = "deserialize_optional_field"
    )]
    pub(crate) mailbox_address: OptionalField<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthMailboxSessionRequest {
    pub(crate) email_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthMailboxStatusRequest {
    #[serde(default)]
    pub(crate) session_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateApiKeyAccountRequest {
    pub(crate) display_name: String,
    pub(crate) email: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) upstream_base_url: Option<String>,
    pub(crate) api_key: String,
    pub(crate) is_mother: Option<bool>,
    pub(crate) local_primary_limit: Option<f64>,
    pub(crate) local_secondary_limit: Option<f64>,
    pub(crate) local_limit_unit: Option<String>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportOauthCredentialFileRequest {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidateImportedOauthAccountsRequest {
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) items: Vec<ImportOauthCredentialFileRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportValidatedOauthAccountsRequest {
    #[serde(default)]
    pub(crate) items: Vec<ImportOauthCredentialFileRequest>,
    #[serde(default)]
    pub(crate) selected_source_ids: Vec<String>,
    #[serde(default)]
    pub(crate) validation_job_id: Option<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthMatchSummary {
    pub(crate) account_id: i64,
    pub(crate) display_name: String,
    pub(crate) group_name: Option<String>,
    pub(crate) status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationRow {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) display_name: Option<String>,
    pub(crate) token_expires_at: Option<String>,
    pub(crate) matched_account: Option<ImportedOauthMatchSummary>,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
    pub(crate) attempts: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationResponse {
    pub(crate) input_files: usize,
    pub(crate) unique_in_input: usize,
    pub(crate) duplicate_in_input: usize,
    pub(crate) rows: Vec<ImportedOauthValidationRow>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationCounts {
    pub(crate) pending: usize,
    pub(crate) duplicate_in_input: usize,
    pub(crate) ok: usize,
    pub(crate) ok_exhausted: usize,
    pub(crate) invalid: usize,
    pub(crate) error: usize,
    pub(crate) checked: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationJobResponse {
    pub(crate) job_id: String,
    pub(crate) snapshot: ImportedOauthValidationResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationSnapshotEvent {
    pub(crate) snapshot: ImportedOauthValidationResponse,
    pub(crate) counts: ImportedOauthValidationCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationRowEvent {
    pub(crate) row: ImportedOauthValidationRow,
    pub(crate) counts: ImportedOauthValidationCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthValidationFailedEvent {
    pub(crate) snapshot: ImportedOauthValidationResponse,
    pub(crate) counts: ImportedOauthValidationCounts,
    pub(crate) error: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ImportedOauthValidationTerminalEvent {
    Completed(ImportedOauthValidationSnapshotEvent),
    Failed(ImportedOauthValidationFailedEvent),
    Cancelled(ImportedOauthValidationSnapshotEvent),
}

#[derive(Debug, Clone)]
pub(crate) enum ImportedOauthValidationJobEvent {
    Row(ImportedOauthValidationRowEvent),
    Completed(ImportedOauthValidationSnapshotEvent),
    Failed(ImportedOauthValidationFailedEvent),
    Cancelled(ImportedOauthValidationSnapshotEvent),
}

#[derive(Debug)]
pub(crate) struct ImportedOauthValidationJob {
    pub(crate) target_group_name: String,
    pub(crate) target_bound_proxy_keys: Vec<String>,
    pub(crate) target_node_shunt_enabled: bool,
    pub(crate) snapshot: Mutex<ImportedOauthValidationResponse>,
    pub(crate) validated_imports: Mutex<HashMap<String, ImportedOauthValidatedImportData>>,
    pub(crate) broadcaster: broadcast::Sender<ImportedOauthValidationJobEvent>,
    pub(crate) cancel: CancellationToken,
    pub(crate) terminal_event: Mutex<Option<ImportedOauthValidationTerminalEvent>>,
}

impl ImportedOauthValidationJob {
    pub(crate) fn new(
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
    pub(crate) account_ids: Vec<i64>,
    pub(crate) action: String,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionResponse {
    pub(crate) action: String,
    pub(crate) requested_count: usize,
    pub(crate) completed_count: usize,
    pub(crate) succeeded_count: usize,
    pub(crate) failed_count: usize,
    pub(crate) results: Vec<BulkUpstreamAccountActionResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountActionResult {
    pub(crate) account_id: i64,
    pub(crate) display_name: Option<String>,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncJobRequest {
    pub(crate) account_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncCounts {
    pub(crate) total: usize,
    pub(crate) completed: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) skipped: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncRow {
    pub(crate) account_id: i64,
    pub(crate) display_name: String,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncSnapshot {
    pub(crate) job_id: String,
    pub(crate) status: String,
    pub(crate) rows: Vec<BulkUpstreamAccountSyncRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncJobResponse {
    pub(crate) job_id: String,
    pub(crate) snapshot: BulkUpstreamAccountSyncSnapshot,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncRowEvent {
    pub(crate) row: BulkUpstreamAccountSyncRow,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncSnapshotEvent {
    pub(crate) snapshot: BulkUpstreamAccountSyncSnapshot,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkUpstreamAccountSyncFailedEvent {
    pub(crate) snapshot: BulkUpstreamAccountSyncSnapshot,
    pub(crate) counts: BulkUpstreamAccountSyncCounts,
    pub(crate) error: String,
}

#[derive(Debug, Clone)]
pub(crate) enum BulkUpstreamAccountSyncTerminalEvent {
    Completed(BulkUpstreamAccountSyncSnapshotEvent),
    Failed(BulkUpstreamAccountSyncFailedEvent),
    Cancelled(BulkUpstreamAccountSyncSnapshotEvent),
}

#[derive(Debug, Clone)]
pub(crate) enum BulkUpstreamAccountSyncJobEvent {
    Row(BulkUpstreamAccountSyncRowEvent),
    Completed(BulkUpstreamAccountSyncSnapshotEvent),
    Failed(BulkUpstreamAccountSyncFailedEvent),
    Cancelled(BulkUpstreamAccountSyncSnapshotEvent),
}

#[derive(Debug)]
pub(crate) struct BulkUpstreamAccountSyncJob {
    pub(crate) snapshot: Mutex<BulkUpstreamAccountSyncSnapshot>,
    pub(crate) broadcaster: broadcast::Sender<BulkUpstreamAccountSyncJobEvent>,
    pub(crate) cancel: CancellationToken,
    pub(crate) terminal_event: Mutex<Option<BulkUpstreamAccountSyncTerminalEvent>>,
}

impl BulkUpstreamAccountSyncJob {
    pub(crate) fn new(snapshot: BulkUpstreamAccountSyncSnapshot) -> Self {
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
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
    pub(crate) account_id: Option<i64>,
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
    pub(crate) matched_account: Option<ImportedOauthMatchSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportSummary {
    pub(crate) input_files: usize,
    pub(crate) selected_files: usize,
    pub(crate) created: usize,
    pub(crate) updated_existing: usize,
    pub(crate) failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedOauthImportResponse {
    pub(crate) summary: ImportedOauthImportSummary,
    pub(crate) results: Vec<ImportedOauthImportResult>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountRequest {
    pub(crate) display_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) email: OptionalField<String>,
    pub(crate) group_name: Option<String>,
    #[serde(default)]
    pub(crate) group_bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) group_node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
    pub(crate) note: Option<String>,
    pub(crate) group_note: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) upstream_base_url: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) bound_proxy_keys: OptionalField<Vec<String>>,
    pub(crate) enabled: Option<bool>,
    pub(crate) is_mother: Option<bool>,
    pub(crate) api_key: Option<String>,
    pub(crate) local_primary_limit: Option<f64>,
    pub(crate) local_secondary_limit: Option<f64>,
    pub(crate) local_limit_unit: Option<String>,
    pub(crate) tag_ids: Option<Vec<i64>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) response_endpoint_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) chat_completions_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) image_endpoint_capability_override: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) response_image_tool_capability_override: OptionalField<String>,
    pub(crate) routing_rule: Option<UpdateGroupAccountRoutingRuleRequest>,
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
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
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
    #[serde(default)]
    pub(crate) group_single_account_rotation_enabled: Option<bool>,
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
    pub(crate) name: String,
    pub(crate) allow_cut_out: bool,
    pub(crate) allow_cut_in: bool,
    pub(crate) priority_tier: Option<String>,
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) upstream_429_retry_enabled: Option<bool>,
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default)]
    pub(crate) available_models: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateTagRequest {
    pub(crate) name: Option<String>,
    pub(crate) allow_cut_out: Option<bool>,
    pub(crate) allow_cut_in: Option<bool>,
    pub(crate) priority_tier: Option<String>,
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) upstream_429_retry_enabled: Option<bool>,
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) available_models: OptionalField<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateGroupAccountRoutingRuleRequest {
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) allow_cut_out: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) allow_cut_in: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) priority_tier: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) fast_mode_rewrite_mode: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) image_tool_rewrite_mode: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) request_compression_algorithm: OptionalField<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) concurrency_limit: OptionalField<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) upstream_429_retry_enabled: OptionalField<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) upstream_429_max_retries: OptionalField<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    pub(crate) available_models: OptionalField<Vec<String>>,
    #[serde(default)]
    pub(crate) status_change_reasons: Option<UpdateStatusChangeReasonSettingsRequest>,
    #[serde(default)]
    pub(crate) timeouts: Option<UpdateRoutingTimeoutSettingsRequest>,
}

impl UpdateGroupAccountRoutingRuleRequest {
    pub(crate) fn priority_tier_value(&self) -> Option<&str> {
        match &self.priority_tier {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn fast_mode_rewrite_mode_value(&self) -> Option<&str> {
        match &self.fast_mode_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn image_tool_rewrite_mode_value(&self) -> Option<&str> {
        match &self.image_tool_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn request_compression_algorithm_value(&self) -> Option<&str> {
        match &self.request_compression_algorithm {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        }
    }

    pub(crate) fn status_change_reason_field(
        &self,
        reason_code: &str,
    ) -> Result<OptionalField<bool>> {
        self.status_change_reasons
            .as_ref()
            .map(|value| value.field(reason_code))
            .transpose()
            .map(|value| value.unwrap_or(OptionalField::Missing))
    }
}

pub(crate) fn optional_bool_to_i64(value: &OptionalField<bool>) -> Option<i64> {
    match value {
        OptionalField::Value(value) => Some(if *value { 1_i64 } else { 0_i64 }),
        OptionalField::Missing | OptionalField::Null => None,
    }
}

pub(crate) fn optional_retry_count_to_i64(value: &OptionalField<u8>) -> Option<i64> {
    match value {
        OptionalField::Value(value) => {
            Some(i64::from(normalize_group_upstream_429_max_retries(*value)))
        }
        OptionalField::Missing | OptionalField::Null => None,
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(transparent)]
pub(crate) struct UpdateStatusChangeReasonSettingsRequest {
    pub(crate) values: BTreeMap<String, Option<bool>>,
}

impl UpdateStatusChangeReasonSettingsRequest {
    fn validate_keys(&self) -> Result<()> {
        for reason_code in self.values.keys() {
            match canonical_status_change_reason_code(reason_code) {
                Some(canonical) if canonical == reason_code => {}
                Some(_) => bail!(
                    "legacy status change reason keys are read-only; use canonical reasonCode values"
                ),
                None => bail!("unknown status change reason: {reason_code}"),
            }
        }
        Ok(())
    }

    fn field(&self, reason_code: &str) -> Result<OptionalField<bool>> {
        self.validate_keys()?;
        let Some(reason_code) = canonical_status_change_reason_code(reason_code) else {
            bail!("unknown status change reason: {reason_code}");
        };
        Ok(match self.values.get(reason_code) {
            Some(Some(value)) => OptionalField::Value(*value),
            Some(None) => OptionalField::Null,
            None => OptionalField::Missing,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListTagsQuery {
    pub(crate) search: Option<String>,
    pub(crate) has_accounts: Option<bool>,
    pub(crate) allow_cut_in: Option<bool>,
    pub(crate) allow_cut_out: Option<bool>,
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
    pub(crate) note: Option<String>,
    #[serde(default)]
    pub(crate) bound_proxy_keys: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) node_shunt_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) single_account_rotation_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_429_retry_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_429_max_retries: Option<u8>,
    pub(crate) concurrency_limit: Option<i64>,
    pub(crate) routing_rule: Option<UpdateGroupAccountRoutingRuleRequest>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OauthCallbackQuery {
    pub(crate) code: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredApiKeyCredentials {
    pub(crate) api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredOauthCredentials {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: String,
    pub(crate) token_type: Option<String>,
}

pub(crate) fn normalize_oauth_refresh_token(value: Option<String>) -> Option<String> {
    normalize_optional_text(value)
}

pub(crate) fn oauth_refresh_token(credentials: &StoredOauthCredentials) -> Option<&str> {
    credentials
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn oauth_credentials_have_refresh_token(credentials: &StoredOauthCredentials) -> bool {
    oauth_refresh_token(credentials).is_some()
}

pub(crate) fn apply_oauth_token_response(
    credentials: &mut StoredOauthCredentials,
    response: OAuthTokenResponse,
) -> String {
    credentials.access_token = response.access_token;
    if let Some(refresh_token) = response.refresh_token {
        credentials.refresh_token = normalize_oauth_refresh_token(Some(refresh_token));
    }
    if let Some(id_token) = response.id_token {
        credentials.id_token = id_token;
    }
    credentials.token_type = response.token_type;
    format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum StoredCredentials {
    ApiKey(StoredApiKeyCredentials),
    Oauth(StoredOauthCredentials),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EncryptedCredentialsPayload {
    pub(crate) v: u8,
    pub(crate) nonce: String,
    pub(crate) ciphertext: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedUsageSnapshot {
    pub(crate) plan_type: Option<String>,
    pub(crate) limit_id: String,
    pub(crate) limit_name: Option<String>,
    pub(crate) primary: Option<NormalizedUsageWindow>,
    pub(crate) secondary: Option<NormalizedUsageWindow>,
    pub(crate) credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedUsageWindow {
    pub(crate) used_percent: f64,
    pub(crate) window_duration_mins: i64,
    pub(crate) resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthTokenResponse {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    #[serde(default)]
    pub(crate) id_token: Option<String>,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    pub(crate) expires_in: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChatgptJwtClaims {
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_plan_type: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImportedOauthCredentialsFile {
    #[serde(rename = "type")]
    #[serde(default)]
    pub(crate) _source_type: Option<serde_json::Value>,
    pub(crate) email: String,
    pub(crate) account_id: String,
    #[serde(default)]
    pub(crate) expired: Option<String>,
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<serde_json::Value>,
    pub(crate) id_token: String,
    #[serde(default)]
    #[serde(rename = "last_refresh")]
    pub(crate) _last_refresh: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) token_type: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedImportedOauthCredentials {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: String,
    pub(crate) display_name: String,
    pub(crate) chatgpt_account_id: String,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) token_expires_at: String,
    pub(crate) credentials: StoredOauthCredentials,
    pub(crate) claims: ChatgptJwtClaims,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthProbeOutcome {
    pub(crate) token_expires_at: String,
    pub(crate) credentials: StoredOauthCredentials,
    pub(crate) claims: ChatgptJwtClaims,
    pub(crate) usage_snapshot: Option<NormalizedUsageSnapshot>,
    pub(crate) maintenance_proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
    pub(crate) exhausted: bool,
    pub(crate) usage_snapshot_warning: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthValidatedImportData {
    pub(crate) normalized: NormalizedImportedOauthCredentials,
    pub(crate) probe: ImportedOauthProbeOutcome,
}

pub(crate) struct PersistOauthCallbackInput {
    pub(crate) session: OauthLoginSessionRow,
    pub(crate) display_name: String,
    pub(crate) chosen_email: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) claims: ChatgptJwtClaims,
    pub(crate) encrypted_credentials: String,
    pub(crate) has_refresh_token: bool,
    pub(crate) token_expires_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtOuterClaims {
    #[serde(default)]
    pub(crate) email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    pub(crate) profile: Option<ChatgptJwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    pub(crate) auth: Option<ChatgptJwtAuthClaims>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JwtExpiryClaims {
    #[serde(default)]
    pub(crate) exp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtProfileClaims {
    #[serde(default)]
    pub(crate) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtAuthClaims {
    #[serde(default)]
    pub(crate) chatgpt_plan_type: Option<String>,
    #[serde(default)]
    pub(crate) chatgpt_user_id: Option<String>,
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) chatgpt_account_id: Option<String>,
}

#[cfg(test)]
mod status_change_reason_tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn default_status_change_reasons_enable_every_reason_code() {
        let settings = default_status_change_reasons();

        assert_eq!(settings.len(), STATUS_CHANGE_REASON_CODES.len());
        for reason_code in STATUS_CHANGE_REASON_CODES {
            assert_eq!(settings.get(reason_code), Some(&true));
        }
    }

    #[test]
    fn legacy_upstream_rejected_alias_maps_to_402_reason() {
        assert_eq!(
            canonical_status_change_reason_code(
                LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED,
            ),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
        );
    }

    #[test]
    fn status_change_reason_patch_rejects_legacy_alias_keys() {
        let request = UpdateStatusChangeReasonSettingsRequest {
            values: BTreeMap::from([(
                LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED.to_string(),
                Some(false),
            )]),
        };

        assert!(request.validate_keys().is_err());
    }
}
