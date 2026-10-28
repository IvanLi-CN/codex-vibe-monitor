use std::{env, fmt, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use sqlx::{PgPool, postgres::PgPoolOptions};

pub(crate) const ENV_POSTGRES_DATABASE_URL: &str = "POSTGRES_DATABASE_URL";
pub(crate) const ENV_POSTGRES_MAX_CONNECTIONS: &str = "POSTGRES_MAX_CONNECTIONS";
pub(crate) const ENV_MIGRATION_SWITCH_BUDGET_MS: &str = "MIGRATION_SWITCH_BUDGET_MS";
pub(crate) const ENV_STORAGE_MIGRATION_ADMIN_TOKEN: &str = "STORAGE_MIGRATION_ADMIN_TOKEN";
pub(crate) const DEFAULT_POSTGRES_MAX_CONNECTIONS: u32 = 5;
pub(crate) const DEFAULT_MIGRATION_SWITCH_BUDGET: Duration = Duration::from_secs(3);

#[derive(Clone, PartialEq, Eq, Serialize)]
pub(crate) struct StorageConfig {
    #[serde(skip_serializing)]
    pub(crate) postgres_database_url: Option<String>,
    #[serde(skip_serializing)]
    pub(crate) migration_admin_token: Option<String>,
    pub(crate) postgres_max_connections: u32,
    pub(crate) migration_switch_budget: Duration,
}

impl fmt::Debug for StorageConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StorageConfig")
            .field(
                "postgres_database_url",
                &self.postgres_database_url.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "migration_admin_token",
                &self.migration_admin_token.as_ref().map(|_| "<redacted>"),
            )
            .field("postgres_max_connections", &self.postgres_max_connections)
            .field("migration_switch_budget", &self.migration_switch_budget)
            .finish()
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            postgres_database_url: None,
            migration_admin_token: None,
            postgres_max_connections: DEFAULT_POSTGRES_MAX_CONNECTIONS,
            migration_switch_budget: DEFAULT_MIGRATION_SWITCH_BUDGET,
        }
    }
}

impl StorageConfig {
    pub(crate) fn postgres_target_identity(&self) -> String {
        let Some(database_url) = self.postgres_database_url.as_deref() else {
            return "postgres:unconfigured".to_string();
        };
        let Ok(url) = url::Url::parse(database_url) else {
            return "postgres:invalid".to_string();
        };
        if !matches!(url.scheme(), "postgres" | "postgresql") {
            return "postgres:invalid".to_string();
        }
        let Some(host) = url.host_str() else {
            return "postgres:invalid".to_string();
        };
        let Some(port) = url.port_or_known_default() else {
            return "postgres:invalid".to_string();
        };
        let database = url.path().trim_start_matches('/');
        if database.is_empty() {
            return "postgres:invalid".to_string();
        }
        let host = host.to_ascii_lowercase();
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        format!("postgres://{host}:{port}/{database}")
    }

    pub(crate) fn from_env() -> Result<Self> {
        let postgres_database_url = match env::var(ENV_POSTGRES_DATABASE_URL) {
            Ok(value) if !value.trim().is_empty() => Some(value),
            Ok(_) => bail!("{ENV_POSTGRES_DATABASE_URL} must not be empty"),
            Err(env::VarError::NotPresent) => None,
            Err(error) => {
                return Err(anyhow!(
                    "failed to read {ENV_POSTGRES_DATABASE_URL}: {error}"
                ));
            }
        };
        let migration_admin_token = match env::var(ENV_STORAGE_MIGRATION_ADMIN_TOKEN) {
            Ok(value) => {
                let value = value.trim();
                if value.len() < 16 || value.chars().any(char::is_whitespace) {
                    bail!(
                        "{ENV_STORAGE_MIGRATION_ADMIN_TOKEN} must contain a nonempty token of at least 16 characters without whitespace"
                    );
                }
                Some(value.to_string())
            }
            Err(env::VarError::NotPresent) => None,
            Err(error) => {
                return Err(anyhow!(
                    "failed to read {ENV_STORAGE_MIGRATION_ADMIN_TOKEN}: {error}"
                ));
            }
        };
        let postgres_max_connections = parse_non_zero_u32(
            ENV_POSTGRES_MAX_CONNECTIONS,
            DEFAULT_POSTGRES_MAX_CONNECTIONS,
        )?;
        let switch_budget_ms = parse_non_zero_u64(
            ENV_MIGRATION_SWITCH_BUDGET_MS,
            DEFAULT_MIGRATION_SWITCH_BUDGET.as_millis() as u64,
        )?;
        let migration_switch_budget = Duration::from_millis(switch_budget_ms);
        if migration_switch_budget.is_zero() {
            bail!("{ENV_MIGRATION_SWITCH_BUDGET_MS} must produce a non-zero duration");
        }
        Ok(Self {
            postgres_database_url,
            migration_admin_token,
            postgres_max_connections,
            migration_switch_budget,
        })
    }

    pub(crate) async fn connect_postgres(&self) -> Result<Option<PgPool>> {
        let Some(database_url) = self.postgres_database_url.as_deref() else {
            return Ok(None);
        };
        PgPoolOptions::new()
            .max_connections(self.postgres_max_connections)
            .connect(database_url)
            .await
            .with_context(|| {
                format!(
                    "failed to connect PostgreSQL storage pool from {ENV_POSTGRES_DATABASE_URL}"
                )
            })
            .map(Some)
    }

    pub(crate) fn connect_postgres_lazy(&self) -> Result<Option<PgPool>> {
        let Some(database_url) = self.postgres_database_url.as_deref() else {
            return Ok(None);
        };
        PgPoolOptions::new()
            .max_connections(self.postgres_max_connections)
            .connect_lazy(database_url)
            .with_context(|| {
                format!(
                    "failed to initialize lazy PostgreSQL storage pool from {ENV_POSTGRES_DATABASE_URL}"
                )
            })
            .map(Some)
    }
}

fn parse_non_zero_u32(name: &str, default_value: u32) -> Result<u32> {
    match env::var(name) {
        Ok(raw) => {
            let value = raw
                .parse::<u32>()
                .with_context(|| format!("invalid {name}: {raw}"))?;
            if value == 0 {
                bail!("{name} must be greater than 0");
            }
            Ok(value)
        }
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(error) => Err(anyhow!("failed to read {name}: {error}")),
    }
}

fn parse_non_zero_u64(name: &str, default_value: u64) -> Result<u64> {
    match env::var(name) {
        Ok(raw) => {
            let value = raw
                .parse::<u64>()
                .with_context(|| format!("invalid {name}: {raw}"))?;
            if value == 0 {
                bail!("{name} must be greater than 0");
            }
            Ok(value)
        }
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(error) => Err(anyhow!("failed to read {name}: {error}")),
    }
}
