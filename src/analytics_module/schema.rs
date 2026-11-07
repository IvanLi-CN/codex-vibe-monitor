use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Connection, PgConnection};
use std::fmt;

#[cfg(test)]
#[path = "schema/postgres.rs"]
mod postgres_tests;

pub(crate) const ANALYTICS_MODULE_VERSION: i64 = 1;
pub(crate) const ANALYTICS_MODULE_READER_VERSION: i64 = 3;
pub(crate) const ANALYTICS_MODULE_DDL_MIN_READER_VERSION: i64 = 1;
pub(crate) const ANALYTICS_MODULE_DDL_COMPONENT: &str = "analytics-module";
pub(crate) const ANALYTICS_MODULE_DDL_STEP: &str = "module-owner-build-publication-v1";
pub(crate) const ANALYTICS_MODULE_SCHEMA_VERSION: i64 = 1;
const ANALYTICS_MODULE_MIGRATION_LOCK_KEY: i64 = 0x0043_564d_5f41_4d44;

pub(crate) const ANALYTICS_ROLE_OWNER: &str = "cvm_analytics_owner";
pub(crate) const ANALYTICS_ROLE_READER: &str = "cvm_analytics_reader";
pub(crate) const ANALYTICS_ROLE_APPLICATION: &str = "cvm_analytics_app";
pub(crate) const ANALYTICS_ROLE_MAINTENANCE: &str = "cvm_analytics_maintenance";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AnalyticsDdlStep {
    pub(crate) order: i64,
    pub(crate) step_id: &'static str,
    pub(crate) min_reader_version: i64,
    pub(crate) requires_bootstrap: bool,
}

pub(crate) const ANALYTICS_MODULE_DDL_STEPS: &[AnalyticsDdlStep] = &[AnalyticsDdlStep {
    order: 1,
    step_id: ANALYTICS_MODULE_DDL_STEP,
    min_reader_version: ANALYTICS_MODULE_DDL_MIN_READER_VERSION,
    requires_bootstrap: true,
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsModuleId {
    Invocation,
    LongTermUsage,
    ParallelWork,
    UpstreamAccountActivity,
    PromptCacheStatistics,
    DashboardActivity,
    Network,
    ForwardProxyStatistics,
}

impl AnalyticsModuleId {
    pub(crate) const ALL: [Self; 8] = [
        Self::Invocation,
        Self::LongTermUsage,
        Self::ParallelWork,
        Self::UpstreamAccountActivity,
        Self::PromptCacheStatistics,
        Self::DashboardActivity,
        Self::Network,
        Self::ForwardProxyStatistics,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Invocation => "invocation",
            Self::LongTermUsage => "long_term_usage",
            Self::ParallelWork => "parallel_work",
            Self::UpstreamAccountActivity => "upstream_account_activity",
            Self::PromptCacheStatistics => "prompt_cache_statistics",
            Self::DashboardActivity => "dashboard_activity",
            Self::Network => "network",
            Self::ForwardProxyStatistics => "forward_proxy_statistics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsConsumerState {
    Building,
    Active,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsOwnerStatus {
    Fenced,
    Claiming,
    Active,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsBuildState {
    Pending,
    Building,
    Ready,
    Failed,
    Retired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsPublicationState {
    Staging,
    Published,
    Superseded,
    Invalidated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsRepairState {
    Pending,
    Running,
    Resolved,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalyticsDdlState {
    Applying,
    Applied,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsModuleDeclaration {
    pub(crate) id: AnalyticsModuleId,
    pub(crate) consumer_id: &'static str,
    pub(crate) version: i64,
    pub(crate) reader_version: i64,
    pub(crate) requires_retention_pin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsConsumer {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) consumer_id: String,
    pub(crate) version: i64,
    pub(crate) state: AnalyticsConsumerState,
    pub(crate) source_instance_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) baseline_change_id: i64,
    pub(crate) checkpoint: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) retention_pin_id: Option<String>,
    pub(crate) active_build_id: Option<String>,
    pub(crate) active_publication_id: Option<String>,
    pub(crate) legacy_reader_fenced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsOwner {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) owner_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) status: AnalyticsOwnerStatus,
    pub(crate) build_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsBuild {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) build_id: String,
    pub(crate) consumer_id: String,
    pub(crate) source_instance_id: String,
    pub(crate) source_generation: i64,
    pub(crate) source_high_watermark: i64,
    pub(crate) source_coverage_revision: i64,
    pub(crate) owner_epoch: i64,
    pub(crate) state: AnalyticsBuildState,
    pub(crate) retention_pin_id: Option<String>,
    pub(crate) projection_version: i64,
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsPublication {
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) publication_id: String,
    pub(crate) consumer_id: String,
    pub(crate) build_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) published_through_change_id: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) projection_version: i64,
    pub(crate) reader_version: i64,
    pub(crate) state: AnalyticsPublicationState,
    pub(crate) legacy_reader_fenced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsRepairObligation {
    pub(crate) repair_id: String,
    pub(crate) module_id: AnalyticsModuleId,
    pub(crate) consumer_id: String,
    pub(crate) build_id: String,
    pub(crate) owner_epoch: i64,
    pub(crate) from_change_id: i64,
    pub(crate) through_change_id: i64,
    pub(crate) reason_code: String,
    pub(crate) state: AnalyticsRepairState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnalyticsDdlLedgerEntry {
    pub(crate) component: String,
    pub(crate) schema_version: i64,
    pub(crate) step_order: i64,
    pub(crate) step_id: String,
    pub(crate) checksum: String,
    pub(crate) state: AnalyticsDdlState,
    pub(crate) min_reader_version: i64,
    pub(crate) requires_bootstrap: bool,
    pub(crate) error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyticsModuleMigrationResult {
    pub(crate) component: &'static str,
    pub(crate) version: i64,
    pub(crate) checksum: String,
    pub(crate) recovered_steps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyticsModuleRoleApplyResult {
    pub(crate) checksum: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalyticsModuleMigrationError {
    TransactionBegin,
    LockAcquire,
    LedgerBootstrap,
    LedgerRead,
    LedgerChecksumMismatch,
    LedgerStepMismatch,
    LedgerNewerVersion,
    LedgerStateInvalid,
    LedgerWrite,
    DdlApply,
    RoleApply,
    Commit,
}

impl AnalyticsModuleMigrationError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::TransactionBegin => "analytics_module_transaction_begin_failed",
            Self::LockAcquire => "analytics_module_migration_lock_failed",
            Self::LedgerBootstrap => "analytics_module_ddl_ledger_bootstrap_failed",
            Self::LedgerRead => "analytics_module_ddl_ledger_read_failed",
            Self::LedgerChecksumMismatch => "analytics_module_ddl_checksum_mismatch",
            Self::LedgerStepMismatch => "analytics_module_ddl_step_mismatch",
            Self::LedgerNewerVersion => "analytics_module_ddl_newer_version",
            Self::LedgerStateInvalid => "analytics_module_ddl_state_invalid",
            Self::LedgerWrite => "analytics_module_ddl_ledger_write_failed",
            Self::DdlApply => "analytics_module_ddl_apply_failed",
            Self::RoleApply => "analytics_module_role_grant_failed",
            Self::Commit => "analytics_module_transaction_commit_failed",
        }
    }
}

impl fmt::Display for AnalyticsModuleMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AnalyticsModuleMigrationError {}

fn analytics_module_schema_checksum() -> String {
    format!(
        "{:x}",
        Sha256::digest(super::ANALYTICS_MODULE_SCHEMA_SQL.as_bytes())
    )
}

async fn acquire_module_migration_lock(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), AnalyticsModuleMigrationError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ANALYTICS_MODULE_MIGRATION_LOCK_KEY)
        .execute(&mut **transaction)
        .await
        .map_err(|_| AnalyticsModuleMigrationError::LockAcquire)?;
    Ok(())
}

/// Apply module DDL after the legacy `pg-analytics` schema has been provisioned.
///
/// The caller must run the old `PG_ANALYTICS_SCHEMA_SQL` contract first; it creates
/// `cvm_analytics`, and this function intentionally does not create that schema.
/// The required order is legacy schema, module schema, then module object grants.
/// This function must run on an owner-authorized migration connection, never on
/// the application or maintenance runtime credentials.
pub(crate) async fn apply_analytics_module_schema(
    connection: &mut PgConnection,
) -> Result<AnalyticsModuleMigrationResult, AnalyticsModuleMigrationError> {
    let mut transaction = connection
        .begin()
        .await
        .map_err(|_| AnalyticsModuleMigrationError::TransactionBegin)?;
    acquire_module_migration_lock(&mut transaction).await?;

    sqlx::raw_sql(super::ANALYTICS_MODULE_LEDGER_SCHEMA_SQL)
        .execute(&mut *transaction)
        .await
        .map_err(|_| AnalyticsModuleMigrationError::LedgerBootstrap)?;

    let checksum = analytics_module_schema_checksum();
    let max_step_order = ANALYTICS_MODULE_DDL_STEPS
        .iter()
        .map(|step| step.order)
        .max()
        .unwrap_or_default();
    let newer_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) \
         FROM cvm_analytics.analytics_module_ddl_ledger \
         WHERE component = $1 \
           AND (schema_version > $2 OR step_order > $3 OR min_reader_version > $4)",
    )
    .bind(ANALYTICS_MODULE_DDL_COMPONENT)
    .bind(ANALYTICS_MODULE_SCHEMA_VERSION)
    .bind(max_step_order)
    .bind(ANALYTICS_MODULE_READER_VERSION)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| AnalyticsModuleMigrationError::LedgerRead)?;
    if newer_rows != 0 {
        return Err(AnalyticsModuleMigrationError::LedgerNewerVersion);
    }

    let mut recovered_steps = 0;
    for step in ANALYTICS_MODULE_DDL_STEPS {
        let existing = sqlx::query_as::<_, (i64, String, String, String)>(
            "SELECT schema_version, step_id, checksum, state \
             FROM cvm_analytics.analytics_module_ddl_ledger \
             WHERE component = $1 AND step_order = $2 \
             FOR UPDATE",
        )
        .bind(ANALYTICS_MODULE_DDL_COMPONENT)
        .bind(step.order)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| AnalyticsModuleMigrationError::LedgerRead)?;

        let existing_state =
            if let Some((schema_version, step_id, stored_checksum, state)) = existing {
                if schema_version > ANALYTICS_MODULE_SCHEMA_VERSION {
                    return Err(AnalyticsModuleMigrationError::LedgerNewerVersion);
                }
                if step_id != step.step_id {
                    return Err(AnalyticsModuleMigrationError::LedgerStepMismatch);
                }
                if stored_checksum != checksum {
                    return Err(AnalyticsModuleMigrationError::LedgerChecksumMismatch);
                }
                if !matches!(state.as_str(), "applying" | "applied" | "failed") {
                    return Err(AnalyticsModuleMigrationError::LedgerStateInvalid);
                }
                Some(state)
            } else {
                None
            };

        if existing_state.as_deref() != Some("applied") {
            sqlx::query(
                "INSERT INTO cvm_analytics.analytics_module_ddl_ledger \
                 (component, schema_version, step_order, step_id, checksum, state, min_reader_version, \
                  requires_bootstrap, error_code, applied_at) \
                 VALUES ($1, $2, $3, $4, $5, 'applying', $6, $7, NULL, NULL) \
                 ON CONFLICT (component, step_order) DO UPDATE SET \
                   schema_version = EXCLUDED.schema_version, step_id = EXCLUDED.step_id, \
                   checksum = EXCLUDED.checksum, \
                   state = 'applying', min_reader_version = EXCLUDED.min_reader_version, \
                   requires_bootstrap = EXCLUDED.requires_bootstrap, error_code = NULL, \
                   applied_at = NULL",
            )
            .bind(ANALYTICS_MODULE_DDL_COMPONENT)
            .bind(ANALYTICS_MODULE_SCHEMA_VERSION)
            .bind(step.order)
            .bind(step.step_id)
            .bind(&checksum)
            .bind(step.min_reader_version)
            .bind(step.requires_bootstrap)
            .execute(&mut *transaction)
            .await
            .map_err(|_| AnalyticsModuleMigrationError::LedgerWrite)?;
            recovered_steps += 1;
        }
    }

    sqlx::raw_sql(super::ANALYTICS_MODULE_SCHEMA_SQL)
        .execute(&mut *transaction)
        .await
        .map_err(|_| AnalyticsModuleMigrationError::DdlApply)?;

    for step in ANALYTICS_MODULE_DDL_STEPS {
        let updated = sqlx::query(
            "UPDATE cvm_analytics.analytics_module_ddl_ledger \
             SET schema_version = $1, checksum = $2, state = 'applied', \
                 error_code = NULL, applied_at = NOW() \
             WHERE component = $3 AND step_order = $4",
        )
        .bind(ANALYTICS_MODULE_SCHEMA_VERSION)
        .bind(&checksum)
        .bind(ANALYTICS_MODULE_DDL_COMPONENT)
        .bind(step.order)
        .execute(&mut *transaction)
        .await
        .map_err(|_| AnalyticsModuleMigrationError::LedgerWrite)?;
        if updated.rows_affected() != 1 {
            return Err(AnalyticsModuleMigrationError::LedgerWrite);
        }
    }

    transaction
        .commit()
        .await
        .map_err(|_| AnalyticsModuleMigrationError::Commit)?;

    Ok(AnalyticsModuleMigrationResult {
        component: ANALYTICS_MODULE_DDL_COMPONENT,
        version: ANALYTICS_MODULE_SCHEMA_VERSION,
        checksum,
        recovered_steps,
    })
}

/// Reconcile module schema/table grants after a successful module DDL commit.
///
/// Execute this only through the temporary migration/DBA connection while it is
/// authorized to act as `cvm_analytics_owner` (normally via controlled `SET ROLE`).
/// The application and maintenance roles must not execute this function. Role
/// creation, credentials, and `cvm_analytics_reader` membership for
/// `cvm_analytics_app` belong to owner-managed provisioning, because a migration
/// login with `NOCREATEROLE` or without role-admin authority cannot grant that
/// membership. This API therefore applies object grants only and does not widen
/// database role authority.
pub(crate) async fn apply_analytics_module_roles(
    connection: &mut PgConnection,
) -> Result<AnalyticsModuleRoleApplyResult, AnalyticsModuleMigrationError> {
    let mut transaction = connection
        .begin()
        .await
        .map_err(|_| AnalyticsModuleMigrationError::TransactionBegin)?;
    acquire_module_migration_lock(&mut transaction).await?;
    sqlx::raw_sql(super::ANALYTICS_MODULE_ROLE_GRANTS_SQL)
        .execute(&mut *transaction)
        .await
        .map_err(|_| AnalyticsModuleMigrationError::RoleApply)?;
    let checksum = format!(
        "{:x}",
        Sha256::digest(super::ANALYTICS_MODULE_ROLE_GRANTS_SQL.as_bytes())
    );
    transaction
        .commit()
        .await
        .map_err(|_| AnalyticsModuleMigrationError::Commit)?;
    Ok(AnalyticsModuleRoleApplyResult { checksum })
}
