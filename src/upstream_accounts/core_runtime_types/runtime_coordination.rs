use super::sync::sync_usage_snapshot::UsageSnapshotVersion;
use super::*;
use futures_util::FutureExt;
use std::{any::Any, panic::AssertUnwindSafe};

#[derive(Debug)]
pub(crate) struct UpstreamAccountsRuntime {
    pub(crate) crypto_key: Option<[u8; 32]>,
    pub(crate) route_facts: Mutex<Option<Arc<RuntimeRouteFacts>>>,
    pub(crate) account_ops: AccountOpCoordinator,
    pub(crate) credential_refresh_locks:
        Arc<std::sync::Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>,
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
    ManualCredentialRefresh,
    ManualUsageRefresh,
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

impl AccountOpCoordinator {
    pub(super) fn new(maintenance_parallelism: usize) -> Self {
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

    pub(super) async fn drain_maintenance_tasks(&self) {
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

    #[expect(
        clippy::too_many_arguments,
        reason = "The actor boundary carries the optimistic-version guards with the existing OAuth upsert command."
    )]
    pub(crate) async fn run_external_oauth_upsert(
        &self,
        state: Arc<AppState>,
        id: i64,
        identity: ExternalAccountIdentity,
        metadata: ExternalUpstreamAccountMetadataRequest,
        expected_updated_at: Option<String>,
        expected_usage_version: Option<Option<UsageSnapshotVersion>>,
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
                    expected_updated_at.as_deref(),
                    expected_usage_version,
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

    pub(crate) async fn run_manual_usage_refresh(
        &self,
        state: Arc<AppState>,
        id: i64,
    ) -> Result<UpstreamAccountDetail, anyhow::Error> {
        self.submit_command(
            state,
            id,
            AccountCommand::ManualUsageRefresh,
            false,
            move |state, id| async move {
                refresh_upstream_account_usage_by_id(state.as_ref(), id).await
            },
        )
        .await
        .map_err(map_account_dispatch_anyhow)
        .and_then(|outcome| match outcome {
            AccountSubmitOutcome::Completed(Some(detail)) => Ok(detail),
            AccountSubmitOutcome::Completed(None) => {
                Err(anyhow!("manual usage refresh returned no detail"))
            }
            AccountSubmitOutcome::Deduped => Err(anyhow!(
                "manual usage refresh was unexpectedly deduplicated"
            )),
        })
    }

    pub(crate) async fn run_manual_credential_refresh(
        &self,
        state: Arc<AppState>,
        id: i64,
    ) -> Result<UpstreamAccountDetail, anyhow::Error> {
        self.submit_command(
            state,
            id,
            AccountCommand::ManualCredentialRefresh,
            false,
            move |state, id| async move {
                refresh_upstream_account_credentials_by_id(state.as_ref(), id).await
            },
        )
        .await
        .map_err(map_account_dispatch_anyhow)
        .and_then(|outcome| match outcome {
            AccountSubmitOutcome::Completed(Some(detail)) => Ok(detail),
            AccountSubmitOutcome::Completed(None) => {
                Err(anyhow!("manual credential refresh returned no detail"))
            }
            AccountSubmitOutcome::Deduped => Err(anyhow!(
                "manual credential refresh was unexpectedly deduplicated"
            )),
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
        self.run_persist_imported_oauth_with_versions(state, id, probe, None, None)
            .await
    }

    pub(crate) async fn run_persist_imported_oauth_with_versions(
        &self,
        state: Arc<AppState>,
        id: i64,
        probe: ImportedOauthProbeOutcome,
        expected_updated_at: Option<String>,
        expected_usage_version: Option<Option<UsageSnapshotVersion>>,
    ) -> Result<Option<String>, (StatusCode, String)> {
        self.submit_command(
            state,
            id,
            AccountCommand::PersistImportedOauth,
            false,
            move |state, id| async move {
                persist_imported_oauth_existing_inner_with_versions(
                    state.as_ref(),
                    id,
                    probe,
                    expected_updated_at.as_deref(),
                    expected_usage_version,
                )
                .await
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
