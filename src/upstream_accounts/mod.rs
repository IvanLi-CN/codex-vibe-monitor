use super::*;
use crate::oauth_bridge::oauth_codex_upstream_base_url;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use axum::{
    extract::{Path as AxumPath, Query},
    http::header,
    response::Html,
};
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use futures_util::FutureExt;
use rand::{Rng, RngCore, rngs::OsRng};
use sqlx::Transaction;
use std::{any::Any, panic::AssertUnwindSafe};
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
const DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM: usize = 4;
const DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS: u64 = 60 * 60;
const DEFAULT_MANUAL_OAUTH_CALLBACK_PORT: u16 = 1455;
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
const LOGIN_SESSION_STATUS_PENDING: &str = "pending";
const LOGIN_SESSION_STATUS_COMPLETED: &str = "completed";
const LOGIN_SESSION_STATUS_FAILED: &str = "failed";
const LOGIN_SESSION_STATUS_EXPIRED: &str = "expired";
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
const USAGE_PATH_STYLE_CHATGPT: &str = "/wham/usage";
const USAGE_PATH_STYLE_CODEX_API: &str = "/api/codex/usage";
const UPSTREAM_USAGE_BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

#[derive(Debug)]
pub(crate) struct UpstreamAccountsRuntime {
    pub(crate) crypto_key: Option<[u8; 32]>,
    account_ops: AccountOpCoordinator,
    validation_jobs: Arc<Mutex<HashMap<String, Arc<ImportedOauthValidationJob>>>>,
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
        id: i64,
    ) -> Result<MaintenanceQueueOutcome, anyhow::Error> {
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
                        sync_upstream_account_by_id(state.as_ref(), id, SyncCause::Maintenance)
                            .await
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
pub(crate) struct UpstreamAccountListResponse {
    writes_enabled: bool,
    items: Vec<UpstreamAccountSummary>,
    groups: Vec<UpstreamAccountGroupSummary>,
    has_ungrouped_accounts: bool,
    routing: PoolRoutingSettingsResponse,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListUpstreamAccountsQuery {
    pub(crate) group_search: Option<String>,
    pub(crate) group_ungrouped: Option<bool>,
    #[serde(default)]
    pub(crate) tag_ids: Vec<i64>,
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
    enabled: bool,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    plan_type: Option<String>,
    masked_api_key: Option<String>,
    last_synced_at: Option<String>,
    last_successful_sync_at: Option<String>,
    last_activity_at: Option<String>,
    last_error: Option<String>,
    last_error_at: Option<String>,
    token_expires_at: Option<String>,
    primary_window: Option<RateWindowSnapshot>,
    secondary_window: Option<RateWindowSnapshot>,
    credits: Option<CreditsSnapshot>,
    local_limits: Option<LocalLimitSnapshot>,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSettingsResponse {
    writes_enabled: bool,
    api_key_configured: bool,
    masked_api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdatePoolRoutingSettingsRequest {
    api_key: String,
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
    account_id: Option<i64>,
    error: Option<String>,
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
    snapshot: Mutex<ImportedOauthValidationResponse>,
    validated_imports: Mutex<HashMap<String, ImportedOauthValidatedImportData>>,
    broadcaster: broadcast::Sender<ImportedOauthValidationJobEvent>,
    cancel: CancellationToken,
    terminal_event: Mutex<Option<ImportedOauthValidationTerminalEvent>>,
}

impl ImportedOauthValidationJob {
    fn new(snapshot: ImportedOauthValidationResponse) -> Self {
        let (broadcaster, _rx) = broadcast::channel(256);
        Self {
            snapshot: Mutex::new(snapshot),
            validated_imports: Mutex::new(HashMap::new()),
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
    expired: String,
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
    last_selected_at: Option<String>,
    last_route_failure_at: Option<String>,
    cooldown_until: Option<String>,
    consecutive_route_failures: i64,
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

#[derive(Debug, FromRow)]
struct AccountRoutingCandidateRow {
    id: i64,
    secondary_used_percent: Option<f64>,
    primary_used_percent: Option<f64>,
    last_selected_at: Option<String>,
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
#[derive(Debug, FromRow)]
struct OauthLoginSessionRow {
    login_id: String,
    account_id: Option<i64>,
    display_name: Option<String>,
    group_name: Option<String>,
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
            last_activity_at TEXT,
            last_selected_at TEXT,
            last_route_failure_at TEXT,
            cooldown_until TEXT,
            consecutive_route_failures INTEGER NOT NULL DEFAULT 0,
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
    ensure_nullable_text_column(pool, "pool_upstream_accounts", "cooldown_until")
        .await
        .context("failed to ensure pool_upstream_accounts.cooldown_until")?;
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
        CREATE TABLE IF NOT EXISTS pool_oauth_login_sessions (
            login_id TEXT PRIMARY KEY,
            account_id INTEGER,
            display_name TEXT,
            group_name TEXT,
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
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_group_notes table existence")?;

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
        CREATE TABLE IF NOT EXISTS pool_routing_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            encrypted_api_key TEXT,
            masked_api_key TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_routing_settings table existence")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pool_routing_settings (
            id,
            encrypted_api_key,
            masked_api_key
        ) VALUES (?1, NULL, NULL)
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
        let mut ticker = interval(state.config.upstream_accounts_sync_interval);
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

async fn sqlite_table_exists(pool: &Pool<Sqlite>, table_name: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await?
        > 0)
}

pub(crate) async fn list_upstream_accounts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListUpstreamAccountsQuery>,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let items = load_upstream_account_summaries_filtered(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    let groups = load_upstream_account_groups(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let has_ungrouped_accounts = has_ungrouped_upstream_accounts(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let routing = load_pool_routing_settings(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        items,
        groups,
        has_ungrouped_accounts,
        routing: PoolRoutingSettingsResponse {
            writes_enabled: state.upstream_accounts.writes_enabled(),
            api_key_configured: routing
                .encrypted_api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            masked_api_key: routing.masked_api_key,
        },
    }))
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

    let mut tx = state.pool.begin().await.map_err(internal_error_tuple)?;
    if !group_has_accounts_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?
    {
        return Err((StatusCode::NOT_FOUND, "group not found".to_string()));
    }
    save_group_note_record_conn(tx.as_mut(), &group_name, note.clone())
        .await
        .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;

    Ok(Json(UpstreamAccountGroupSummary { group_name, note }))
}

pub(crate) async fn get_upstream_account(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let detail = load_upstream_account_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}

async fn build_imported_oauth_validation_response(
    state: &AppState,
    items: &[ImportOauthCredentialFileRequest],
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
            build_imported_oauth_validation_result(state, normalized)
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

async fn build_imported_oauth_validation_result(
    state: &AppState,
    normalized: NormalizedImportedOauthCredentials,
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

    match probe_imported_oauth_credentials(state, &normalized).await {
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

fn spawn_imported_oauth_validation_job(
    state: Arc<AppState>,
    runtime: Arc<UpstreamAccountsRuntime>,
    job_id: String,
    items: Vec<ImportOauthCredentialFileRequest>,
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
                async move {
                    (
                        row_index,
                        build_imported_oauth_validation_result(state.as_ref(), normalized).await,
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
    let snapshot = build_imported_oauth_pending_response(&payload.items);
    let job_id = random_hex(16)?;
    let job = Arc::new(ImportedOauthValidationJob::new(snapshot.clone()));
    state
        .upstream_accounts
        .insert_validation_job(job_id.clone(), job.clone())
        .await;
    spawn_imported_oauth_validation_job(
        state.clone(),
        state.upstream_accounts.clone(),
        job_id.clone(),
        payload.items,
        job,
    );
    Ok(Json(ImportedOauthValidationJobResponse {
        job_id,
        snapshot,
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
    Ok(Json(
        build_imported_oauth_validation_response(state.as_ref(), &payload.items).await,
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
    let tag_ids = validate_tag_ids(&state.pool, &tag_ids).await?;
    let cached_validation_results = if let Some(job_id) = normalize_optional_text(validation_job_id)
    {
        if let Some(job) = state.upstream_accounts.get_validation_job(&job_id).await {
            job.validated_imports.lock().await.clone()
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
            None => match probe_imported_oauth_credentials(state.as_ref(), &normalized).await {
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
        let (persisted_account_id, import_warning) =
            if let Some(existing_row) = existing_match.as_ref() {
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
                            group_note: group_note.clone(),
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
    let row = load_pool_routing_settings(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(PoolRoutingSettingsResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        api_key_configured: row
            .encrypted_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        masked_api_key: row.masked_api_key,
    }))
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
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let api_key = normalize_required_secret(&payload.api_key, "apiKey")?;
    save_pool_routing_api_key(&state.pool, crypto_key, &api_key)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(PoolRoutingSettingsResponse {
        writes_enabled: true,
        api_key_configured: true,
        masked_api_key: Some(mask_api_key(&api_key)),
    }))
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
        let supported_domains = moemail_config
            .email_domains
            .unwrap_or_default()
            .split(',')
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
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
        let remote_mailbox = moemail_list_emails(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?
            .into_iter()
            .find(|item| {
                normalize_mailbox_address(&item.address) == Some(manual_email_address.clone())
            });
        let Some(remote_mailbox) = remote_mailbox else {
            return Ok(Json(oauth_mailbox_session_unsupported_response(
                manual_email_address,
                "not_readable",
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
            login_id, account_id, display_name, group_name, is_mother, note, tag_ids_json, group_note,
            mailbox_session_id, generated_mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, expires_at, consumed_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, ?16, NULL, ?17, ?17)
        "#,
    )
    .bind(&login_id)
    .bind(payload.account_id)
    .bind(display_name)
    .bind(group_name)
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
        account_id: payload.account_id,
        error: None,
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
    let detail = load_upstream_account_detail(&state.pool, account_id)
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
    validate_group_note_target(group_name.as_deref(), has_group_note)?;
    let target_group_name = group_name.clone();
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
    .bind(&group_name)
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

        save_group_note_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            group_note,
            has_group_note,
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
    let tag_ids = match payload.tag_ids.as_ref() {
        Some(values) => Some(validate_tag_ids(&state.pool, values).await?),
        None => None,
    };
    let previous_group_name = row.group_name.clone();
    let requested_group_note = payload
        .group_note
        .clone()
        .map(|value| normalize_optional_text(Some(value)));

    if let Some(display_name) = payload.display_name {
        row.display_name = normalize_required_display_name(&display_name)?;
    }
    if let Some(group_name) = payload.group_name {
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

    if let Some(group_note) = requested_group_note {
        save_group_note_after_account_write(
            tx.as_mut(),
            row.group_name.as_deref(),
            group_note,
            true,
            previous_group_name == row.group_name,
        )
        .await
        .map_err(internal_error_tuple)?;
    }
    if previous_group_name != row.group_name {
        cleanup_orphaned_group_note(tx.as_mut(), previous_group_name.as_deref())
            .await
            .map_err(internal_error_tuple)?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Some(tag_ids) = tag_ids {
        sync_account_tag_links(&state.pool, id, &tag_ids)
            .await
            .map_err(internal_error_tuple)?;
    }

    let detail = load_upstream_account_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(detail)
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
    cleanup_orphaned_group_note(
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

    let token_response = exchange_authorization_code(
        &state.http_clients.shared,
        &state.config,
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
            group_note: session.group_note.clone(),
            claims: &input.claims,
            encrypted_credentials: input.encrypted_credentials,
            token_expires_at: &input.token_expires_at,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    complete_login_session_with_executor(&mut *tx, &session.login_id, account_id)
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

    let account_ids = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE kind = ?1 AND enabled = 1
        ORDER BY updated_at ASC, id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .fetch_all(&state.pool)
    .await?;

    let mut queued = 0usize;
    let mut deduped = 0usize;
    let mut failed = 0usize;
    for account_id in account_ids {
        match state
            .upstream_accounts
            .account_ops
            .dispatch_maintenance_sync(state.clone(), account_id)
        {
            Ok(MaintenanceQueueOutcome::Queued) => queued += 1,
            Ok(MaintenanceQueueOutcome::Deduped) => deduped += 1,
            Err(err) => {
                failed += 1;
                warn!(account_id, error = %err, "failed to dispatch upstream OAuth maintenance");
            }
        }
    }

    info!(
        candidates = queued + deduped + failed,
        queued, deduped, failed, "upstream account maintenance pass finished"
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

fn should_maintain_account(row: &UpstreamAccountRow, state: &AppState) -> bool {
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX || row.enabled == 0 {
        return false;
    }
    if row.status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH {
        return false;
    }
    let now = Utc::now();
    let sync_due = row
        .last_synced_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|last| {
            now.signed_duration_since(last).num_seconds()
                >= state.config.upstream_accounts_sync_interval.as_secs() as i64
        })
        .unwrap_or(true);
    let refresh_due = row
        .token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|expires| {
            expires
                <= now
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);
    sync_due || refresh_due || row.status == UPSTREAM_ACCOUNT_STATUS_ERROR
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
            last_selected_at, last_route_failure_at, cooldown_until, consecutive_route_failures,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url,
            created_at, updated_at
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
            last_selected_at, last_route_failure_at, cooldown_until, consecutive_route_failures,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url,
            created_at, updated_at
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
) -> Result<ImportedOauthProbeOutcome, anyhow::Error> {
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
        let response = refresh_oauth_tokens(
            &state.http_clients.shared,
            &state.config,
            &credentials.refresh_token,
        )
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

    let usage_result = fetch_usage_snapshot(
        &state.http_clients.shared,
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
            let response = refresh_oauth_tokens(
                &state.http_clients.shared,
                &state.config,
                &credentials.refresh_token,
            )
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
            match fetch_usage_snapshot(
                &state.http_clients.shared,
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
        let detail = load_upstream_account_detail(&state.pool, id)
            .await?
            .ok_or_else(|| anyhow!("account not found"))?;
        return Ok(Some(detail));
    }

    if cause == SyncCause::Maintenance && !should_maintain_account(&row, state) {
        return Ok(None);
    }

    match row.kind.as_str() {
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX => sync_oauth_account(state, &row).await?,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX => sync_api_key_account(&state.pool, &row).await?,
        _ => bail!("unsupported account kind: {}", row.kind),
    }

    let detail = load_upstream_account_detail(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found after sync"))?;
    Ok(Some(detail))
}

async fn sync_api_key_account(pool: &Pool<Sqlite>, row: &UpstreamAccountRow) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
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
    .bind(row.id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn sync_oauth_account(state: &AppState, row: &UpstreamAccountRow) -> Result<()> {
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

    if refresh_due {
        match refresh_oauth_tokens(
            &state.http_clients.shared,
            &state.config,
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
                update_account_error(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    &err.to_string(),
                )
                .await?;
                return Ok(());
            }
            Err(err) => {
                update_account_error(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_ERROR,
                    &err.to_string(),
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

    let usage_result = fetch_usage_snapshot(
        &state.http_clients.shared,
        &state.config,
        &credentials.access_token,
        latest_row.chatgpt_account_id.as_deref(),
    )
    .await;

    let snapshot = match usage_result {
        Ok(snapshot) => snapshot,
        Err(err) if err.to_string().contains("401") || err.to_string().contains("403") => {
            match refresh_oauth_tokens(
                &state.http_clients.shared,
                &state.config,
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
                    fetch_usage_snapshot(
                        &state.http_clients.shared,
                        &state.config,
                        &refreshed.access_token,
                        latest_row.chatgpt_account_id.as_deref(),
                    )
                    .await?
                }
                Err(refresh_err) if is_reauth_error(&refresh_err) => {
                    update_account_error(
                        &state.pool,
                        row.id,
                        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                        &refresh_err.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
                Err(refresh_err) => {
                    update_account_error(
                        &state.pool,
                        row.id,
                        UPSTREAM_ACCOUNT_STATUS_ERROR,
                        &refresh_err.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
        Err(err) => {
            update_account_error(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_STATUS_ERROR,
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
    mark_account_sync_success(&state.pool, row.id).await?;
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
        mark_account_sync_success(&state.pool, account_id).await?;
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
            group_note: None,
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
    group_note: Option<String>,
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
        group_note,
        claims,
        encrypted_credentials,
        token_expires_at,
    } = payload;
    let target_group_name = group_name.clone();
    let group_note_was_requested = group_note.is_some();
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
        save_group_note_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            group_note,
            group_note_was_requested,
            previous_group_name == target_group_name,
        )
        .await?;
        if previous_group_name != target_group_name {
            cleanup_orphaned_group_note(tx.as_mut(), previous_group_name.as_deref()).await?;
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
        save_group_note_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            group_note,
            group_note_was_requested,
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
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT groups.group_name, notes.note
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
        .map(|(group_name, note)| UpstreamAccountGroupSummary { group_name, note })
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
            last_selected_at, last_route_failure_at, cooldown_until, consecutive_route_failures,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url,
            created_at, updated_at
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
    let account_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let tag_map = load_account_tag_map(pool, &account_ids).await?;

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
        ));
    }
    Ok(items)
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

    let duplicate_info_map = load_duplicate_info_map(pool).await?;
    Ok(Some(UpstreamAccountDetail {
        summary: build_summary_from_row(
            &row,
            latest.as_ref(),
            row.last_activity_at.clone(),
            tags,
            duplicate_info_map.get(&row.id).cloned(),
        ),
        note: row.note,
        upstream_base_url: row.upstream_base_url,
        chatgpt_user_id: row.chatgpt_user_id,
        last_refreshed_at: row.last_refreshed_at,
        history,
    }))
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
            last_selected_at, last_route_failure_at, cooldown_until, consecutive_route_failures,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url,
            created_at, updated_at
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

    UpstreamAccountSummary {
        id: row.id,
        kind: row.kind.clone(),
        provider: row.provider.clone(),
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        is_mother: row.is_mother != 0,
        status: effective_account_status(row),
        enabled: row.enabled != 0,
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
        last_error: row.last_error.clone(),
        last_error_at: row.last_error_at.clone(),
        token_expires_at: row.token_expires_at.clone(),
        primary_window,
        secondary_window,
        credits,
        local_limits,
        duplicate_info,
        tags,
        effective_routing_rule,
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
    if let Some(note) = note {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_group_notes (group_name, note, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?3)
            ON CONFLICT(group_name) DO UPDATE SET
                note = excluded.note,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(group_name)
        .bind(note)
        .bind(now_iso)
        .execute(conn)
        .await?;
    } else {
        sqlx::query(
            r#"
            DELETE FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
        )
        .bind(group_name)
        .execute(conn)
        .await?;
    }
    Ok(())
}

async fn save_group_note_after_account_write(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    note: Option<String>,
    note_was_requested: bool,
    target_group_already_had_current_account: bool,
) -> Result<()> {
    if !note_was_requested {
        return Ok(());
    }
    let Some(group_name) = group_name else {
        return Ok(());
    };
    if target_group_already_had_current_account {
        return Ok(());
    }
    if group_account_count_conn(conn, group_name).await? != 1 {
        return Ok(());
    }
    save_group_note_record_conn(conn, group_name, note).await
}

async fn cleanup_orphaned_group_note(
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
            login_id, account_id, display_name, group_name, is_mother, note, tag_ids_json, group_note,
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
            login_id, account_id, display_name, group_name, is_mother, note, tag_ids_json, group_note,
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
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            account_id = ?3,
            consumed_at = ?4,
            updated_at = ?4
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_COMPLETED)
    .bind(account_id)
    .bind(&now_iso)
    .execute(executor)
    .await?;
    Ok(())
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
        account_id: row.account_id,
        error: row.error_message.clone(),
    }
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

static OAUTH_SUBJECT_CODE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)your\s+(?:chatgpt|openai)\s+code\s+is\s+(\d{4,8})")
        .expect("valid oauth subject code regex")
});
static OAUTH_BODY_CODE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(?:verification\s+code|code\s+is)[^0-9]{0,24}(\d{4,8})")
        .expect("valid oauth body code regex")
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

fn parse_mailbox_code(detail: &MoeMailMessageDetail) -> Option<ParsedMailboxCode> {
    let subject = detail.subject.as_deref().unwrap_or_default();
    if let Some(captures) = OAUTH_SUBJECT_CODE_REGEX.captures(subject) {
        let value = captures.get(1)?.as_str().to_string();
        return Some(ParsedMailboxCode {
            value,
            source: "subject".to_string(),
            updated_at: detail
                .received_at
                .clone()
                .unwrap_or_else(|| format_utc_iso(Utc::now())),
        });
    }

    for (source, raw) in [
        ("content", detail.content.as_deref().unwrap_or_default()),
        ("html", detail.html.as_deref().unwrap_or_default()),
    ] {
        let normalized = if source == "html" {
            strip_html_tags(raw)
        } else {
            raw.to_string()
        };
        if let Some(captures) = OAUTH_BODY_CODE_REGEX.captures(&normalized) {
            let value = captures.get(1)?.as_str().to_string();
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
    if subject.is_empty() || !subject.to_ascii_lowercase().contains("has invited you") {
        return None;
    }

    let text_candidates = [
        detail.content.as_deref().unwrap_or_default().to_string(),
        strip_html_tags(detail.html.as_deref().unwrap_or_default()),
    ];
    let body = text_candidates.join("\n");
    let normalized = body.to_ascii_lowercase();
    if !normalized.contains("join workspace") && !normalized.contains("accept invitation") {
        return None;
    }

    let copy_value = URL_REGEX
        .find_iter(&body)
        .map(|value| value.as_str().trim_end_matches('.').to_string())
        .find(|value| {
            let lower = value.to_ascii_lowercase();
            lower.contains("workspace") || lower.contains("invite") || lower.contains("accept")
        })?;

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
            "domain": config.default_domain,
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
    })
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
            last_error = ?3,
            last_error_at = CASE WHEN ?3 IS NULL THEN last_error_at ELSE ?4 END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(last_error)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_account_error(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    error_message: &str,
) -> Result<()> {
    set_account_status(pool, account_id, status, Some(error_message)).await
}

async fn mark_account_sync_success(pool: &Pool<Sqlite>, account_id: i64) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
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
    Ok(())
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

fn parse_chatgpt_jwt_claims(id_token: &str) -> Result<ChatgptJwtClaims> {
    let mut parts = id_token.split('.');
    let (_header, payload, _sig) = match (parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(sig))
            if !header.is_empty() && !payload.is_empty() && !sig.is_empty() =>
        {
            (header, payload, sig)
        }
        _ => bail!("invalid id_token format"),
    };
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| BASE64_STANDARD.decode(payload))
        .context("failed to decode id_token payload")?;
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

fn imported_snapshot_is_exhausted(snapshot: &NormalizedUsageSnapshot) -> bool {
    let primary_exhausted = snapshot
        .primary
        .as_ref()
        .is_some_and(|window| window.used_percent >= 100.0);
    let secondary_exhausted = snapshot
        .secondary
        .as_ref()
        .is_some_and(|window| window.used_percent >= 100.0);
    let credits_exhausted = snapshot.credits.as_ref().is_some_and(|credits| {
        credits.has_credits
            && !credits.unlimited
            && credits
                .balance
                .as_deref()
                .and_then(|value| value.parse::<f64>().ok())
                .is_some_and(|value| value <= 0.0)
    });
    primary_exhausted || secondary_exhausted || credits_exhausted
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
    let token_expires_at = parse_rfc3339_utc(&parsed.expired)
        .map(format_utc_iso)
        .ok_or_else(|| "expired must be a valid RFC3339 timestamp".to_string())?;
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

fn is_scope_permission_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("missing scopes")
        || msg.contains("insufficient permissions for this operation")
        || msg.contains("api.responses.write")
        || msg.contains("api.model.read")
}

fn is_bridge_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("oauth bridge")
        || msg.contains("token exchange failed")
        || msg.contains("bridge upstream")
        || msg.contains("bridge token")
}

fn is_explicit_reauth_error_message(message: &str) -> bool {
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

async fn load_pool_routing_settings(pool: &Pool<Sqlite>) -> Result<PoolRoutingSettingsRow> {
    sqlx::query_as::<_, PoolRoutingSettingsRow>(
        r#"
        SELECT encrypted_api_key, masked_api_key
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
    pub(crate) upstream_base_url: Url,
}

#[derive(Debug, Clone)]
pub(crate) enum PoolAccountResolution {
    Resolved(PoolResolvedAccount),
    NoCandidate,
    BlockedByPolicy(String),
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

pub(crate) async fn resolve_pool_account_for_request(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
) -> Result<PoolAccountResolution> {
    let mut tried = excluded_ids.iter().copied().collect::<HashSet<_>>();

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
            && is_account_selectable_for_routing(&row)
        {
            tried.insert(route.account_id);
            if let Some(account) = prepare_pool_account(state, &row).await? {
                return Ok(PoolAccountResolution::Resolved(account));
            }
        }
        if sticky_source_rule
            .as_ref()
            .is_some_and(|rule| !rule.allow_cut_out)
        {
            return Ok(PoolAccountResolution::BlockedByPolicy(
                "sticky conversation cannot cut out of the current account because a tag rule forbids it"
                    .to_string(),
            ));
        }
    }

    let mut candidates = load_account_routing_candidates(&state.pool, &tried).await?;
    candidates.sort_by(compare_routing_candidates);
    for candidate in candidates {
        let Some(row) = load_upstream_account_row(&state.pool, candidate.id).await? else {
            continue;
        };
        if !is_account_selectable_for_routing(&row) {
            continue;
        }
        let effective_rule = load_effective_routing_rule_for_account(&state.pool, row.id).await?;
        if !account_accepts_sticky_assignment(
            &state.pool,
            row.id,
            sticky_key,
            sticky_source_id,
            &effective_rule,
        )
        .await?
        {
            continue;
        }
        if let Some(account) = prepare_pool_account(state, &row).await? {
            return Ok(PoolAccountResolution::Resolved(account));
        }
    }

    Ok(PoolAccountResolution::NoCandidate)
}

pub(crate) async fn record_pool_route_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_selected_at = COALESCE(last_selected_at, ?3),
            last_error = NULL,
            last_error_at = NULL,
            last_route_failure_at = NULL,
            cooldown_until = NULL,
            consecutive_route_failures = 0,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    if let Some(sticky_key) = sticky_key {
        upsert_sticky_route(pool, sticky_key, account_id, &now_iso).await?;
    }
    Ok(())
}

pub(crate) async fn record_pool_route_http_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
) -> Result<()> {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        if let Some(sticky_key) = sticky_key {
            delete_sticky_route(pool, sticky_key).await?;
        }
        let next_status = if account_kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message)
        {
            UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
        } else {
            UPSTREAM_ACCOUNT_STATUS_ERROR
        };
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                cooldown_until = NULL,
                consecutive_route_failures = consecutive_route_failures + 1,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(next_status)
        .bind(error_message)
        .bind(now_iso)
        .execute(pool)
        .await?;
        return Ok(());
    }

    let base_secs = if status == StatusCode::TOO_MANY_REQUESTS {
        15
    } else {
        5
    };
    apply_pool_route_cooldown_failure(pool, account_id, sticky_key, error_message, base_secs).await
}

pub(crate) async fn record_pool_route_transport_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
) -> Result<()> {
    apply_pool_route_cooldown_failure(pool, account_id, sticky_key, error_message, 5).await
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
) -> Result<Option<PoolResolvedAccount>> {
    let Some(crypto_key) = state.upstream_accounts.crypto_key.as_ref() else {
        return Ok(None);
    };
    let Some(encrypted_credentials) = row.encrypted_credentials.as_deref() else {
        return Ok(None);
    };
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
            upstream_base_url,
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
                match refresh_oauth_tokens(
                    &state.http_clients.shared,
                    &state.config,
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
                        update_account_error(
                            &state.pool,
                            row.id,
                            UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                            &err.to_string(),
                        )
                        .await?;
                        return Ok(None);
                    }
                    Err(err) => {
                        update_account_error(
                            &state.pool,
                            row.id,
                            UPSTREAM_ACCOUNT_STATUS_ERROR,
                            &err.to_string(),
                        )
                        .await?;
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
                upstream_base_url,
            }))
        }
    }
}

fn is_account_selectable_for_routing(row: &UpstreamAccountRow) -> bool {
    if row.provider != UPSTREAM_ACCOUNT_PROVIDER_CODEX
        || row.enabled == 0
        || row.status != UPSTREAM_ACCOUNT_STATUS_ACTIVE
        || row.encrypted_credentials.is_none()
    {
        return false;
    }
    let Some(cooldown_until) = row.cooldown_until.as_deref() else {
        return true;
    };
    parse_rfc3339_utc(cooldown_until)
        .map(|until| until <= Utc::now())
        .unwrap_or(true)
}

async fn load_account_routing_candidates(
    pool: &Pool<Sqlite>,
    excluded_ids: &HashSet<i64>,
) -> Result<Vec<AccountRoutingCandidateRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            account.last_selected_at
        FROM pool_upstream_accounts account
        WHERE account.provider = 
        "#,
    );
    query
        .push_bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .push(" AND account.enabled = 1")
        .push(" AND account.status = ")
        .push_bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .push(" AND account.encrypted_credentials IS NOT NULL");
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

fn compare_routing_candidates(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
) -> std::cmp::Ordering {
    let lhs_secondary = lhs.secondary_used_percent.unwrap_or(0.0);
    let rhs_secondary = rhs.secondary_used_percent.unwrap_or(0.0);
    lhs_secondary
        .partial_cmp(&rhs_secondary)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            lhs.primary_used_percent
                .unwrap_or(0.0)
                .partial_cmp(&rhs.primary_used_percent.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
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

async fn apply_pool_route_cooldown_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    base_secs: i64,
) -> Result<()> {
    if let Some(sticky_key) = sticky_key {
        delete_sticky_route(pool, sticky_key).await?;
    }
    let row = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    let next_failures = row.consecutive_route_failures.max(0) + 1;
    let exponent = (next_failures - 1).clamp(0, 5) as u32;
    let cooldown_secs = (base_secs * (1_i64 << exponent)).min(300);
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
            cooldown_until = ?5,
            consecutive_route_failures = ?6,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(error_message)
    .bind(&now_iso)
    .bind(cooldown_until)
    .bind(next_failures)
    .execute(pool)
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
        Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::get,
    };
    use sqlx::SqlitePool;
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{Arc, atomic::AtomicUsize},
        time::Duration,
    };
    use tokio::{
        net::TcpListener,
        sync::{Mutex, Notify},
        time::timeout,
    };

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
                last_selected_at: None,
                last_route_failure_at: None,
                cooldown_until: None,
                consecutive_route_failures: 0,
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
            archive_dir: PathBuf::from("target/archive-tests"),
            invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
            invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
            forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
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
        let config = usage_snapshot_test_config(base_url, "codex-vibe-monitor/test");
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
                },
            )),
            hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
            upstream_accounts: Arc::new(
                UpstreamAccountsRuntime::test_instance_with_maintenance_parallelism(
                    maintenance_parallelism,
                ),
            ),
        })
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
        let payload = json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": plan_type,
                "chatgpt_user_id": chatgpt_user_id,
                "chatgpt_account_id": chatgpt_account_id,
            }
        });
        let encoded = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{encoded}.{body}.{encoded}")
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
                ?1, ?2, ?3, NULL, NULL, ?4, 1, ?5, ?6,
                ?7, ?8, NULL, ?9, ?10,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?11, ?11
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
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
                ?1, ?2, ?3, NULL, NULL, ?4, 1, ?5, ?6,
                ?7, ?8, NULL, ?9, ?10,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?11, ?11
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
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
                group_note: None,
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
                group_note: None,
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
                    group_note: None,
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
                    group_note: None,
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
                    group_note: None,
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
                    group_note: None,
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
                group_note: None,
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
                group_note: None,
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
                group_note: None,
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
                group_note: None,
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
                    group_note: None,
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
                    group_note: None,
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
                    group_note: None,
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
                    group_note: None,
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
                group_note: None,
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
                group_note: None,
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
}
