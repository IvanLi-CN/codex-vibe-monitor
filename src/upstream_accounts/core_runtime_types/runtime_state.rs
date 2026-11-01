use super::*;

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
            route_facts: Mutex::new(None),
            account_ops: AccountOpCoordinator::default(),
            credential_refresh_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            validation_jobs: Arc::new(Mutex::new(HashMap::new())),
            bulk_sync_jobs: Arc::new(Mutex::new(HashMap::new())),
            bulk_sync_creation: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn writes_enabled(&self) -> bool {
        self.crypto_key.is_some()
    }

    pub(crate) fn credential_refresh_lock(&self, account_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .credential_refresh_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry(account_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
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
            route_facts: Mutex::new(None),
            account_ops: AccountOpCoordinator::new(maintenance_parallelism),
            credential_refresh_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
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
