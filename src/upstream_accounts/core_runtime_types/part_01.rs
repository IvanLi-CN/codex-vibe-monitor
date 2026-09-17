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
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED: &str = "model_route_degraded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_COOLDOWN: &str = "model_route_cooldown";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED: &str = "model_route_recovered";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET: &str = "model_route_reset";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING: &str =
    "model_route_cache_observation_missing";
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
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_RECENT_UPSTREAM_STREAM_ERRORS: &str =
    "recent_upstream_stream_errors";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_RECENT_UPSTREAM_STREAM_ERRORS_MESSAGE: &str =
    "近期连续上游流错误，自动路由将在五分钟后恢复";
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
    UpdateModelMappings,
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

    pub(crate) async fn run_update_model_mappings(
        &self,
        state: Arc<AppState>,
        id: i64,
        payload: UpdateModelMappingsRequest,
    ) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::UpdateModelMappings,
            false,
            move |state, id| async move {
                update_upstream_account_model_mappings_inner(state.as_ref(), id, payload).await
            },
        )
        .await
        .map_err(map_account_dispatch_http)?
        .expect_completed(AccountCommand::UpdateModelMappings)
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
