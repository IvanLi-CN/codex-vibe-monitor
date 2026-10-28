//! SQLx-backed migration adapters.
//!
//! This module owns bounded database mechanics. Business modules provide one
//! `MigrationTableSpec` per logical table, including the physical columns used for
//! stable identity, source version, and serialized payload. That keeps traffic and
//! archive schema ownership in pg03/pg04 instead of guessing it here.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use sqlx::{
    PgPool, Postgres, Row, Sqlite, SqlitePool, Transaction, postgres::PgArguments, query::Query,
    sqlite::SqliteRow,
};

use super::{
    catalog::{ModuleCatalog, ModuleId, NativeColumnType, NativeMigrationCatalog, NativeTableSpec},
    core::StorageModule,
    migration::{
        CaptureCheckpoint, CapturedChange, ChangeBatchLimits, ChangeInput, ChangeKind, ChunkProof,
        ChunkRange, CopyLimits, CoverageCheckpoint, DEFAULT_MAX_CAPTURE_BYTES, EngineCheckpoint,
        INITIAL_AUTHORITY_GENERATION, INITIAL_CAPTURE_GENERATION, MigrationSource, MigrationTarget,
        SourcePage, SourceRangePage, SourceRangeSnapshot, SourceWriteAuthority, TableCopyPlan,
        TableCoverageProof, VersionedRow, select_change_batch, validate_checkpoint_progress,
    },
    native::{NativeRow, NativeValue},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationTableSpec {
    pub(crate) module: StorageModule,
    pub(crate) logical_table_name: String,
    pub(crate) source_table_name: String,
    pub(crate) target_table_name: String,
    pub(crate) source_key_column: String,
    pub(crate) source_version_column: String,
    pub(crate) source_payload_column: String,
    pub(crate) target_key_column: String,
    pub(crate) target_version_column: String,
    pub(crate) target_payload_column: String,
    pub(crate) target_schema: MigrationTableSchema,
    pub(crate) native: Option<NativeTableSpec>,
}

impl MigrationTableSpec {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        module: StorageModule,
        logical_table_name: impl Into<String>,
        source_table_name: impl Into<String>,
        target_table_name: impl Into<String>,
        source_key_column: impl Into<String>,
        source_version_column: impl Into<String>,
        source_payload_column: impl Into<String>,
        target_key_column: impl Into<String>,
        target_version_column: impl Into<String>,
        target_payload_column: impl Into<String>,
    ) -> Result<Self> {
        let spec = Self {
            module,
            logical_table_name: logical_table_name.into(),
            source_table_name: source_table_name.into(),
            target_table_name: target_table_name.into(),
            source_key_column: source_key_column.into(),
            source_version_column: source_version_column.into(),
            source_payload_column: source_payload_column.into(),
            target_key_column: target_key_column.into(),
            target_version_column: target_version_column.into(),
            target_payload_column: target_payload_column.into(),
            target_schema: MigrationTableSchema::default(),
            native: None,
        };
        for (value, label) in [
            (&spec.logical_table_name, "logical migration table"),
            (&spec.source_table_name, "source migration table"),
            (&spec.target_table_name, "target migration table"),
            (&spec.source_key_column, "source stable-key column"),
            (&spec.source_version_column, "source version column"),
            (&spec.source_payload_column, "source payload column"),
            (&spec.target_key_column, "target stable-key column"),
            (&spec.target_version_column, "target version column"),
            (&spec.target_payload_column, "target payload column"),
        ] {
            validate_identifier(value, label)?;
        }
        Ok(spec)
    }

    pub(crate) fn with_target_schema(mut self, target_schema: MigrationTableSchema) -> Self {
        self.target_schema = target_schema;
        self
    }

    pub(crate) fn canonical(module: StorageModule, table_name: impl Into<String>) -> Result<Self> {
        let table_name = table_name.into();
        Self::new(
            module,
            table_name.clone(),
            table_name.clone(),
            table_name,
            "stable_key",
            "source_version",
            "payload",
            "stable_key",
            "source_version",
            "payload",
        )
    }

    pub(crate) fn native(module: StorageModule, native: NativeTableSpec) -> Result<Self> {
        let table_name = native.table_name().to_owned();
        if native.stable_key_columns().is_empty() {
            bail!("native migration table {table_name} has no stable key");
        }
        let mut spec = Self::new(
            module,
            table_name.clone(),
            table_name.clone(),
            table_name,
            "stable_key",
            "source_version",
            "payload",
            "stable_key",
            "source_version",
            "payload",
        )?;
        spec.native = Some(native);
        Ok(spec)
    }
}

fn validate_identifier(identifier: &str, label: &str) -> Result<()> {
    let mut chars = identifier.chars();
    let Some(first) = chars.next() else {
        bail!("{label} must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || chars.any(|character| !(character == '_' || character.is_ascii_alphanumeric()))
    {
        bail!("{label} is not a safe SQL identifier: {identifier}");
    }
    Ok(())
}

fn quote_identifier(identifier: &str, label: &str) -> Result<String> {
    validate_identifier(identifier, label)?;
    Ok(format!("\"{identifier}\""))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MigrationColumnType {
    Text,
    BigInt,
    Bytea,
}

impl MigrationColumnType {
    fn postgres_name(self) -> &'static str {
        match self {
            Self::Text => "TEXT",
            Self::BigInt => "BIGINT",
            Self::Bytea => "BYTEA",
        }
    }

    fn information_schema_name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::BigInt => "bigint",
            Self::Bytea => "bytea",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MigrationTableSchema {
    pub(crate) key_type: MigrationColumnType,
    pub(crate) version_type: MigrationColumnType,
    pub(crate) payload_type: MigrationColumnType,
}

impl Default for MigrationTableSchema {
    fn default() -> Self {
        Self {
            key_type: MigrationColumnType::Text,
            version_type: MigrationColumnType::BigInt,
            payload_type: MigrationColumnType::Bytea,
        }
    }
}

fn spec_map(specs: Vec<MigrationTableSpec>) -> Result<BTreeMap<String, MigrationTableSpec>> {
    if specs.is_empty() {
        bail!("SQLx migration adapters require at least one table specification");
    }
    let mut map = BTreeMap::new();
    for spec in specs {
        if map.insert(spec.logical_table_name.clone(), spec).is_some() {
            bail!("SQLx migration adapters received a duplicate table specification");
        }
    }
    Ok(map)
}

fn module_id(module: StorageModule) -> ModuleId {
    match module {
        StorageModule::TrafficArchive => ModuleId::TrafficArchive,
        StorageModule::Analytics => ModuleId::Analytics,
        StorageModule::AccountRouting => ModuleId::AccountRouting,
        StorageModule::TaskOperations => ModuleId::TaskOperations,
        StorageModule::Performance => ModuleId::Performance,
    }
}

fn module_table_names(
    specs: &BTreeMap<String, MigrationTableSpec>,
    module: StorageModule,
) -> Vec<String> {
    specs
        .values()
        .filter(|spec| spec.module == module)
        .map(|spec| spec.logical_table_name.clone())
        .collect()
}

fn ordered_native_module_table_names(
    specs: &BTreeMap<String, MigrationTableSpec>,
    module: StorageModule,
) -> Result<Vec<String>> {
    let mut remaining = specs
        .values()
        .filter(|spec| spec.module == module)
        .map(|spec| spec.logical_table_name.clone())
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let next = remaining.iter().find(|table_name| {
            let Some(native) = specs.get(*table_name).and_then(|spec| spec.native.as_ref()) else {
                return true;
            };
            native.foreign_keys.iter().all(|foreign_key| {
                foreign_key.table == (*table_name).as_str()
                    && (!remaining.contains(&foreign_key.referenced_table)
                        || foreign_key.referenced_table == (*table_name).as_str())
            })
        });
        let Some(next) = next.cloned() else {
            bail!(
                "native migration table set contains a foreign-key cycle: {}",
                remaining.into_iter().collect::<Vec<_>>().join(", ")
            );
        };
        remaining.remove(&next);
        ordered.push(next);
    }
    Ok(ordered)
}

fn ensure_catalog_specs(
    catalog: &ModuleCatalog,
    specs: &BTreeMap<String, MigrationTableSpec>,
    module: StorageModule,
) -> Result<()> {
    let expected = catalog
        .owners()
        .iter()
        .filter(|owner| owner.module == module_id(module))
        .map(|owner| owner.table.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let actual = module_table_names(specs, module)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if expected != actual {
        bail!("SQLx migration specs do not cover the complete SQLite module catalog");
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct SqliteMigrationSource {
    pool: SqlitePool,
    migration_id: String,
    source_identity: String,
    capture_table_name: String,
    capture_state_table_name: String,
    max_capture_bytes: u64,
    writer_generation: u64,
    specs: Arc<BTreeMap<String, MigrationTableSpec>>,
}

#[derive(Debug, Clone, Copy)]
struct CaptureContinuityError;

impl std::fmt::Display for CaptureContinuityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SQLite migration capture continuity has been lost")
    }
}

impl std::error::Error for CaptureContinuityError {}

#[derive(Debug, Clone, Copy)]
struct SqliteCaptureState {
    capture_generation: u64,
    next_version: i64,
    captured_bytes: u64,
    max_capture_bytes: u64,
    capture_capacity_held: bool,
    continuity_lost: bool,
    baseline_required: bool,
    authority_generation: u64,
    source_writes_fenced: bool,
}

impl SqliteMigrationSource {
    pub(crate) fn new(
        pool: SqlitePool,
        migration_id: impl Into<String>,
        source_identity: impl Into<String>,
        capture_table_name: impl Into<String>,
        specs: Vec<MigrationTableSpec>,
    ) -> Result<Self> {
        let migration_id = migration_id.into();
        let source_identity = source_identity.into();
        let capture_table_name = capture_table_name.into();
        if migration_id.trim().is_empty() || source_identity.trim().is_empty() {
            bail!("SQLite migration source requires migration and source identities");
        }
        validate_identifier(&capture_table_name, "SQLite migration capture table")?;
        let capture_state_table_name = format!("{capture_table_name}_state");
        validate_identifier(
            &capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        Ok(Self {
            pool,
            migration_id,
            source_identity,
            capture_table_name,
            capture_state_table_name,
            max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            writer_generation: INITIAL_AUTHORITY_GENERATION,
            specs: Arc::new(spec_map(specs)?),
        })
    }

    pub(crate) fn with_max_capture_bytes(mut self, max_capture_bytes: u64) -> Result<Self> {
        if max_capture_bytes == 0 {
            bail!("SQLite migration capture capacity must be positive");
        }
        self.max_capture_bytes = max_capture_bytes;
        Ok(self)
    }

    pub(crate) fn with_writer_generation(mut self, writer_generation: u64) -> Result<Self> {
        if writer_generation == 0 {
            bail!("SQLite migration writer generation must be non-zero");
        }
        self.writer_generation = writer_generation;
        Ok(self)
    }

    pub(crate) fn from_catalog(
        pool: SqlitePool,
        migration_id: impl Into<String>,
        source_identity: impl Into<String>,
        capture_table_name: impl Into<String>,
        catalog: &ModuleCatalog,
        specs: Vec<MigrationTableSpec>,
        module: StorageModule,
    ) -> Result<Self> {
        let source = Self::new(
            pool,
            migration_id,
            source_identity,
            capture_table_name,
            specs,
        )?;
        ensure_catalog_specs(catalog, &source.specs, module)?;
        Ok(source)
    }

    pub(crate) fn from_native_catalog(
        pool: SqlitePool,
        migration_id: impl Into<String>,
        source_identity: impl Into<String>,
        capture_table_name: impl Into<String>,
        catalog: &NativeMigrationCatalog,
        module: StorageModule,
    ) -> Result<Self> {
        let specs = catalog
            .tables
            .iter()
            .filter(|spec| {
                spec.owner.module
                    == match module {
                        StorageModule::TrafficArchive => ModuleId::TrafficArchive,
                        StorageModule::Analytics => ModuleId::Analytics,
                        StorageModule::AccountRouting => ModuleId::AccountRouting,
                        StorageModule::TaskOperations => ModuleId::TaskOperations,
                        StorageModule::Performance => ModuleId::Performance,
                    }
            })
            .map(|spec| MigrationTableSpec::native(module, spec.clone()))
            .collect::<Result<Vec<_>>>()?;
        if specs.is_empty() {
            bail!("native migration catalog has no tables for {module:?}");
        }
        Self::new(
            pool,
            migration_id,
            source_identity,
            capture_table_name,
            specs,
        )
    }

    pub(crate) async fn ensure_capture_schema(&self) -> Result<()> {
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let index_name = quote_identifier(
            &format!("{}_identity_version", self.capture_table_name),
            "SQLite migration capture index",
        )?;
        let query = format!(
            "CREATE TABLE IF NOT EXISTS {capture_table} (
                capture_id INTEGER PRIMARY KEY AUTOINCREMENT,
                migration_id TEXT NOT NULL,
                source_identity TEXT NOT NULL,
                source_version INTEGER NOT NULL,
                table_name TEXT NOT NULL,
                stable_key TEXT NOT NULL,
                previous_key TEXT,
                kind TEXT NOT NULL,
                payload BLOB,
                payload_digest TEXT NOT NULL
            )"
        );
        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .context("create SQLite migration capture table")?;
        let query = format!(
            "CREATE INDEX IF NOT EXISTS {index_name}
               ON {capture_table} (migration_id, source_identity, source_version, capture_id)"
        );
        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .context("index SQLite migration capture table")?;
        let capture_state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "CREATE TABLE IF NOT EXISTS {capture_state_table} (
                migration_id TEXT NOT NULL,
                source_identity TEXT NOT NULL,
                capture_generation INTEGER NOT NULL,
                next_version INTEGER NOT NULL,
                captured_bytes INTEGER NOT NULL,
                max_capture_bytes INTEGER NOT NULL,
                capture_capacity_held INTEGER NOT NULL DEFAULT 0,
                continuity_lost INTEGER NOT NULL,
                baseline_required INTEGER NOT NULL,
                authority_generation INTEGER NOT NULL,
                source_writes_fenced INTEGER NOT NULL,
                PRIMARY KEY (migration_id, source_identity)
            )"
        );
        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .context("create SQLite migration capture state table")?;
        let has_capacity_hold_column = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info({}) WHERE name = 'capture_capacity_held'
            )",
            sqlite_string_literal(&self.capture_state_table_name),
        ))
        .fetch_one(&self.pool)
        .await?
            != 0;
        if !has_capacity_hold_column {
            sqlx::query(&format!(
                "ALTER TABLE {capture_state_table}
                 ADD COLUMN capture_capacity_held INTEGER NOT NULL DEFAULT 0"
            ))
            .execute(&self.pool)
            .await
            .context("upgrade SQLite migration capture capacity state")?;
        }
        let query = format!(
            "INSERT OR IGNORE INTO {capture_state_table}
                (migration_id, source_identity, capture_generation, next_version,
                 captured_bytes, max_capture_bytes, continuity_lost, baseline_required,
                 authority_generation, source_writes_fenced)
             SELECT ?, ?, ?, COALESCE(MAX(source_version), 0),
                    COALESCE(SUM(COALESCE(length(payload), 0)), 0), ?,
                    CASE
                        WHEN COUNT(*) = 0 THEN 0
                        WHEN COUNT(DISTINCT source_version) = MAX(source_version) - MIN(source_version) + 1 THEN 0
                        ELSE 1
                    END,
                    CASE
                        WHEN COUNT(*) = 0 THEN 0
                        WHEN COUNT(DISTINCT source_version) = MAX(source_version) - MIN(source_version) + 1 THEN 0
                        ELSE 1
                    END,
                    ?, ?
               FROM {capture_table}
              WHERE migration_id = ? AND source_identity = ?"
        );
        sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .bind(i64::try_from(INITIAL_CAPTURE_GENERATION).context("capture generation overflow")?)
            .bind(i64::try_from(self.max_capture_bytes).context("capture capacity overflow")?)
            .bind(
                i64::try_from(INITIAL_AUTHORITY_GENERATION)
                    .context("authority generation overflow")?,
            )
            .bind(0_i64)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .execute(&self.pool)
            .await
            .context("initialize SQLite migration capture state")?;
        Ok(())
    }

    /// Install database-level capture and fence triggers for every native table owned by this
    /// source. This closes legacy SQL writers that cannot be changed to call an application
    /// helper, while retaining the same durable capture checkpoint used by the engine.
    pub(crate) async fn ensure_native_capture_triggers(&self) -> Result<()> {
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        for spec in self.specs.values().filter_map(|spec| spec.native.as_ref()) {
            let table = quote_identifier(spec.table_name(), "native SQLite source table")?;
            let trigger_prefix = format!(
                "migration_capture_{}",
                spec.table_name()
                    .chars()
                    .map(|character| {
                        if character.is_ascii_alphanumeric() || character == '_' {
                            character
                        } else {
                            '_'
                        }
                    })
                    .collect::<String>()
            );
            let fence_body = format!(
                "BEGIN
                    SELECT CASE WHEN EXISTS (
                        SELECT 1 FROM {state_table}
                         WHERE migration_id = {migration_id}
                           AND source_identity = {source_identity}
                           AND source_writes_fenced <> 0
                    ) THEN RAISE(ABORT, 'SQLite source writes are fenced') END;
                END",
                migration_id = sqlite_string_literal(&self.migration_id),
                source_identity = sqlite_string_literal(&self.source_identity),
            );
            for operation in ["INSERT", "UPDATE", "DELETE"] {
                let trigger = quote_identifier(
                    &format!("{trigger_prefix}_fence_{}", operation.to_ascii_lowercase()),
                    "native SQLite fence trigger",
                )?;
                let query = format!(
                    "CREATE TRIGGER IF NOT EXISTS {trigger}
                     BEFORE {operation} ON {table}
                     FOR EACH ROW {fence_body}"
                );
                sqlx::query(&query)
                    .execute(&self.pool)
                    .await
                    .with_context(|| format!("install native SQLite {operation} fence trigger"))?;
            }

            let insert_payload = sqlite_native_capture_payload(spec, "NEW");
            let delete_payload = "NULL".to_string();
            let insert_key = sqlite_native_key_expression(spec, "NEW");
            let delete_key = sqlite_native_key_expression(spec, "OLD");
            let update_key = sqlite_native_key_expression(spec, "NEW");
            let previous_key = sqlite_native_key_expression(spec, "OLD");
            let insert_body = native_capture_trigger_body(
                &capture_table,
                &state_table,
                &self.migration_id,
                &self.source_identity,
                spec.table_name(),
                &insert_key,
                "NULL",
                "insert",
                &insert_payload,
            );
            let update_body = native_capture_trigger_body(
                &capture_table,
                &state_table,
                &self.migration_id,
                &self.source_identity,
                spec.table_name(),
                &update_key,
                "NULL",
                "update",
                &insert_payload,
            );
            let replace_body = native_capture_trigger_body(
                &capture_table,
                &state_table,
                &self.migration_id,
                &self.source_identity,
                spec.table_name(),
                &update_key,
                &previous_key,
                "replace",
                &insert_payload,
            );
            let delete_body = native_capture_trigger_body(
                &capture_table,
                &state_table,
                &self.migration_id,
                &self.source_identity,
                spec.table_name(),
                &delete_key,
                "NULL",
                "delete",
                &delete_payload,
            );
            let update_predicate = format!(" WHEN ({previous_key}) = ({update_key})");
            let replace_predicate = format!(" WHEN ({previous_key}) <> ({update_key})");
            for (suffix, event, predicate, body) in [
                ("insert", "INSERT", "", insert_body),
                ("update", "UPDATE", update_predicate.as_str(), update_body),
                (
                    "replace",
                    "UPDATE",
                    replace_predicate.as_str(),
                    replace_body,
                ),
                ("delete", "DELETE", "", delete_body),
            ] {
                let trigger = quote_identifier(
                    &format!("{trigger_prefix}_{suffix}"),
                    "native SQLite capture trigger",
                )?;
                let query = format!(
                    "CREATE TRIGGER IF NOT EXISTS {trigger}
                     AFTER {event} ON {table}
                     FOR EACH ROW{predicate} {body}"
                );
                sqlx::query(&query)
                    .execute(&self.pool)
                    .await
                    .with_context(|| format!("install native SQLite {event} capture trigger"))?;
            }
        }
        Ok(())
    }

    pub(crate) async fn append_captured_change(&self, change: &CapturedChange) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        let result = self
            .append_captured_change_tx_for_generation(
                &mut transaction,
                self.writer_generation,
                change,
            )
            .await;
        match result {
            Ok(()) => {
                transaction
                    .commit()
                    .await
                    .context("commit SQLite migration change capture")?;
                Ok(())
            }
            Err(error) => {
                transaction
                    .rollback()
                    .await
                    .context("rollback SQLite migration change capture")?;
                if error.downcast_ref::<CaptureContinuityError>().is_some() {
                    self.mark_continuity_lost().await?;
                }
                Err(error)
            }
        }
    }

    pub(crate) async fn append_change(
        &self,
        source_version: i64,
        input: ChangeInput,
    ) -> Result<CapturedChange> {
        self.append_change_for_generation(self.writer_generation, source_version, input)
            .await
    }

    pub(crate) async fn append_change_for_generation(
        &self,
        expected_generation: u64,
        source_version: i64,
        input: ChangeInput,
    ) -> Result<CapturedChange> {
        let change = CapturedChange::from_input(
            &self.migration_id,
            &self.source_identity,
            source_version,
            input,
        )?;
        let mut transaction = self.pool.begin().await?;
        let result = self
            .append_captured_change_tx_for_generation(
                &mut transaction,
                expected_generation,
                &change,
            )
            .await;
        match result {
            Ok(()) => {
                transaction
                    .commit()
                    .await
                    .context("commit SQLite migration change capture")?;
            }
            Err(error) => {
                transaction
                    .rollback()
                    .await
                    .context("rollback SQLite migration change capture")?;
                if error.downcast_ref::<CaptureContinuityError>().is_some() {
                    self.mark_continuity_lost().await?;
                }
                return Err(error);
            }
        }
        Ok(change)
    }

    /// Capture one source mutation in the caller's business transaction.
    ///
    /// The source version is allocated from the durable capture checkpoint while the caller's
    /// transaction is still open. The source-row mutation and this capture row therefore commit
    /// or roll back together; callers must abort the transaction when this method returns an
    /// error.
    pub(crate) async fn capture_mutation_tx(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        input: ChangeInput,
    ) -> Result<CapturedChange> {
        self.capture_mutation_tx_for_generation(transaction, self.writer_generation, input)
            .await
    }

    pub(crate) async fn capture_mutation_tx_for_generation(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        expected_generation: u64,
        input: ChangeInput,
    ) -> Result<CapturedChange> {
        let state = self.load_capture_state_tx(transaction).await?;
        if state.continuity_lost {
            bail!("SQLite migration capture continuity has been lost");
        }
        if state.source_writes_fenced || state.authority_generation != expected_generation {
            bail!("SQLite migration writer generation is stale or source writes are fenced");
        }
        let source_version = state
            .next_version
            .checked_add(1)
            .ok_or_else(|| anyhow!("SQLite migration source version exhausted"))?;
        let change = CapturedChange::from_input(
            &self.migration_id,
            &self.source_identity,
            source_version,
            input,
        )?;
        self.append_captured_change_tx_for_generation(transaction, expected_generation, &change)
            .await?;
        Ok(change)
    }

    pub(crate) async fn append_captured_change_tx(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        change: &CapturedChange,
    ) -> Result<()> {
        self.append_captured_change_tx_for_generation(transaction, self.writer_generation, change)
            .await
    }

    pub(crate) async fn append_captured_change_tx_for_generation(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        expected_generation: u64,
        change: &CapturedChange,
    ) -> Result<()> {
        if expected_generation == 0 {
            bail!("SQLite migration writer generation must be non-zero");
        }
        change.validate_for(&self.migration_id, &self.source_identity)?;
        let state = self.load_capture_state_tx(transaction).await?;
        if state.continuity_lost {
            bail!("SQLite migration capture continuity has been lost");
        }
        if state.source_writes_fenced || state.authority_generation != expected_generation {
            bail!("SQLite migration writer generation is stale or source writes are fenced");
        }
        let has_capture_rows = self.capture_has_rows_tx(transaction).await?;
        let next_version = if !has_capture_rows {
            change.source_version
        } else {
            state
                .next_version
                .checked_add(1)
                .ok_or_else(|| anyhow!("SQLite migration source version exhausted"))?
        };
        if has_capture_rows
            && (change.source_version < state.next_version || change.source_version > next_version)
        {
            self.mark_continuity_lost_tx(transaction).await?;
            return Err(anyhow!(CaptureContinuityError));
        }
        let payload_bytes = change
            .payload
            .as_ref()
            .map_or(0, |payload| payload.len() as u64);
        let captured_bytes = state
            .captured_bytes
            .checked_add(payload_bytes)
            .ok_or_else(|| anyhow!("SQLite migration capture byte count overflowed"))?;
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query = format!(
            "INSERT INTO {capture_table}
             (migration_id, source_identity, source_version, table_name, stable_key,
              previous_key, kind, payload, payload_digest)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)"
        );
        sqlx::query(&query)
            .bind(&change.migration_id)
            .bind(&change.source_identity)
            .bind(change.source_version)
            .bind(&change.table_name)
            .bind(&change.stable_key)
            .bind(&change.previous_key)
            .bind(change_kind_name(change.kind))
            .bind(&change.payload)
            .bind(&change.payload_digest)
            .execute(&mut **transaction)
            .await
            .context("append SQLite migration change capture")?;
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET next_version = ?, captured_bytes = ?,
                    capture_capacity_held = CASE
                        WHEN ? > max_capture_bytes THEN 1
                        ELSE capture_capacity_held
                    END
              WHERE migration_id = ?
                AND source_identity = ?
                AND authority_generation = ?
                AND source_writes_fenced = 0
                AND continuity_lost = 0"
        );
        let result = sqlx::query(&query)
            .bind(state.next_version.max(change.source_version))
            .bind(
                i64::try_from(captured_bytes)
                    .context("SQLite migration capture bytes overflowed")?,
            )
            .bind(
                i64::try_from(captured_bytes)
                    .context("SQLite migration capture bytes overflowed")?,
            )
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .bind(i64::try_from(expected_generation).context("writer generation overflow")?)
            .execute(&mut **transaction)
            .await
            .context("update SQLite migration capture checkpoint")?;
        if result.rows_affected() != 1 {
            bail!("SQLite migration writer generation changed during capture");
        }
        Ok(())
    }

    async fn capture_has_rows_tx(&self, transaction: &mut Transaction<'_, Sqlite>) -> Result<bool> {
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query = format!(
            "SELECT EXISTS(
                    SELECT 1
                      FROM {capture_table}
                     WHERE migration_id = ? AND source_identity = ?
                ) AS has_rows"
        );
        sqlx::query_scalar(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .fetch_one(&mut **transaction)
            .await
            .context("check SQLite migration capture continuity")
    }

    pub(crate) async fn release_capture_through(&self, through: i64) -> Result<u64> {
        if through < 0 {
            bail!("SQLite migration capture release watermark must not be negative");
        }
        let mut transaction = self.pool.begin().await?;
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let deleted = sqlx::query(&format!(
            "DELETE FROM {capture_table}
              WHERE migration_id = ? AND source_identity = ? AND source_version <= ?"
        ))
        .bind(&self.migration_id)
        .bind(&self.source_identity)
        .bind(through)
        .execute(&mut *transaction)
        .await
        .context("release consumed SQLite migration capture")?
        .rows_affected();
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        sqlx::query(&format!(
            "UPDATE {state_table}
                SET captured_bytes = COALESCE((
                        SELECT SUM(COALESCE(length(payload), 0))
                          FROM {capture_table}
                         WHERE migration_id = ? AND source_identity = ?
                    ), 0),
                    capture_capacity_held = CASE
                        WHEN COALESCE((
                            SELECT SUM(COALESCE(length(payload), 0))
                              FROM {capture_table}
                             WHERE migration_id = ? AND source_identity = ?
                        ), 0) <= max_capture_bytes THEN 0
                        ELSE 1
                    END
              WHERE migration_id = ? AND source_identity = ?"
        ))
        .bind(&self.migration_id)
        .bind(&self.source_identity)
        .bind(&self.migration_id)
        .bind(&self.source_identity)
        .bind(&self.migration_id)
        .bind(&self.source_identity)
        .execute(&mut *transaction)
        .await
        .context("release SQLite migration capture capacity")?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration capture release")?;
        Ok(deleted)
    }

    async fn load_capture_state_tx(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
    ) -> Result<SqliteCaptureState> {
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "SELECT capture_generation, next_version, captured_bytes, max_capture_bytes,
                    capture_capacity_held,
                    continuity_lost, baseline_required, authority_generation,
                    source_writes_fenced
               FROM {state_table}
              WHERE migration_id = ? AND source_identity = ?"
        );
        let row = sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .fetch_one(&mut **transaction)
            .await
            .context("load SQLite migration capture checkpoint")?;
        let state = SqliteCaptureState {
            capture_generation: u64::try_from(row.try_get::<i64, _>("capture_generation")?)
                .context("SQLite migration capture generation is negative")?,
            next_version: row.try_get("next_version")?,
            captured_bytes: u64::try_from(row.try_get::<i64, _>("captured_bytes")?)
                .context("SQLite migration capture bytes are negative")?,
            max_capture_bytes: u64::try_from(row.try_get::<i64, _>("max_capture_bytes")?)
                .context("SQLite migration capture capacity is negative")?,
            capture_capacity_held: row.try_get::<i64, _>("capture_capacity_held")? != 0,
            continuity_lost: row.try_get::<i64, _>("continuity_lost")? != 0,
            baseline_required: row.try_get::<i64, _>("baseline_required")? != 0,
            authority_generation: u64::try_from(row.try_get::<i64, _>("authority_generation")?)
                .context("SQLite migration authority generation is negative")?,
            source_writes_fenced: row.try_get::<i64, _>("source_writes_fenced")? != 0,
        };
        if state.capture_generation == 0
            || state.next_version < 0
            || state.max_capture_bytes == 0
            || state.authority_generation == 0
        {
            bail!("SQLite migration capture checkpoint is invalid");
        }
        Ok(state)
    }

    pub(crate) async fn capture_checkpoint(&self) -> Result<CaptureCheckpoint> {
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        let checkpoint = CaptureCheckpoint {
            migration_id: self.migration_id.clone(),
            source_identity: self.source_identity.clone(),
            capture_generation: state.capture_generation,
            next_version: state.next_version,
            captured_bytes: state.captured_bytes,
            max_capture_bytes: state.max_capture_bytes,
            continuity_lost: state.continuity_lost,
            baseline_required: state.baseline_required,
        };
        checkpoint.validate()?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration capture checkpoint read")?;
        Ok(checkpoint)
    }

    pub(crate) async fn capture_capacity_held(&self) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration capture capacity read")?;
        Ok(state.capture_capacity_held)
    }

    pub(crate) async fn mark_continuity_lost(&self) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        self.mark_continuity_lost_tx(&mut transaction).await?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration capture continuity loss")?;
        Ok(())
    }

    async fn mark_continuity_lost_tx(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
    ) -> Result<()> {
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET continuity_lost = 1, baseline_required = 1
              WHERE migration_id = ? AND source_identity = ?"
        );
        let result = sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .execute(&mut **transaction)
            .await
            .context("mark SQLite migration capture continuity lost")?;
        if result.rows_affected() != 1 {
            bail!("SQLite migration capture checkpoint is missing");
        }
        Ok(())
    }

    pub(crate) async fn start_new_capture_generation(&self) -> Result<CaptureCheckpoint> {
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        if state.source_writes_fenced {
            bail!("cannot start a capture generation after source authority was fenced");
        }
        let capture_generation = state
            .capture_generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("SQLite migration capture generation exhausted"))?;
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query = format!(
            "DELETE FROM {capture_table}
              WHERE migration_id = ? AND source_identity = ?"
        );
        sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .execute(&mut *transaction)
            .await
            .context("reset SQLite migration capture generation")?;
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET capture_generation = ?, captured_bytes = 0,
                    capture_capacity_held = 0,
                    continuity_lost = 0, baseline_required = 1
              WHERE migration_id = ? AND source_identity = ?"
        );
        sqlx::query(&query)
            .bind(i64::try_from(capture_generation).context("capture generation overflow")?)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .execute(&mut *transaction)
            .await
            .context("persist SQLite migration capture generation")?;
        let checkpoint = CaptureCheckpoint {
            migration_id: self.migration_id.clone(),
            source_identity: self.source_identity.clone(),
            capture_generation,
            next_version: state.next_version,
            captured_bytes: 0,
            max_capture_bytes: state.max_capture_bytes,
            continuity_lost: false,
            baseline_required: true,
        };
        checkpoint.validate()?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration capture generation")?;
        Ok(checkpoint)
    }

    pub(crate) async fn mark_baseline_complete(
        &self,
        capture_generation: u64,
        through: i64,
    ) -> Result<()> {
        if capture_generation == 0 || through < 0 {
            bail!("SQLite migration baseline checkpoint is invalid");
        }
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        if state.capture_generation != capture_generation {
            bail!("capture baseline belongs to another generation");
        }
        if state.continuity_lost || through > state.next_version {
            bail!("capture baseline watermark is outside the current capture");
        }
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET baseline_required = 0
              WHERE migration_id = ? AND source_identity = ?"
        );
        sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .execute(&mut *transaction)
            .await
            .context("complete SQLite migration capture baseline")?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration capture baseline")?;
        Ok(())
    }

    pub(crate) async fn source_write_authority(&self) -> Result<SourceWriteAuthority> {
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        let authority = SourceWriteAuthority {
            generation: state.authority_generation,
            fenced: state.source_writes_fenced,
        };
        authority.validate()?;
        transaction
            .commit()
            .await
            .context("commit SQLite source authority read")?;
        Ok(authority)
    }

    pub(crate) async fn fence_source_writes(
        &self,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<()> {
        if expected_generation == 0 || next_generation <= expected_generation {
            bail!("source authority generations must advance monotonically");
        }
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        if state.source_writes_fenced {
            bail!("source writes are already durably fenced");
        }
        if state.authority_generation != expected_generation {
            bail!("source writer generation is stale");
        }
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET authority_generation = ?, source_writes_fenced = 1
              WHERE migration_id = ?
                AND source_identity = ?
                AND authority_generation = ?
                AND source_writes_fenced = 0"
        );
        let result = sqlx::query(&query)
            .bind(i64::try_from(next_generation).context("authority generation overflow")?)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .bind(i64::try_from(expected_generation).context("authority generation overflow")?)
            .execute(&mut *transaction)
            .await
            .context("persist SQLite source write fence")?;
        if result.rows_affected() != 1 {
            bail!("SQLite source write authority changed during fencing");
        }
        transaction
            .commit()
            .await
            .context("commit SQLite source write fence")?;
        Ok(())
    }

    pub(crate) async fn reopen_source_writes(&self, expected_generation: u64) -> Result<()> {
        if expected_generation == 0 {
            bail!("source authority generation must be non-zero");
        }
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        if state.authority_generation != expected_generation {
            bail!(
                "source writer generation is stale during reopen: expected {expected_generation}, current {}",
                state.authority_generation
            );
        }
        if !state.source_writes_fenced {
            transaction
                .commit()
                .await
                .context("commit idempotent SQLite source write reopen")?;
            return Ok(());
        }
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET source_writes_fenced = 0
              WHERE migration_id = ?
                AND source_identity = ?
                AND authority_generation = ?
                AND source_writes_fenced = 1"
        );
        let result = sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .bind(i64::try_from(expected_generation).context("authority generation overflow")?)
            .execute(&mut *transaction)
            .await
            .context("reopen SQLite source writes")?;
        if result.rows_affected() != 1 {
            bail!("SQLite source write authority changed during reopen");
        }
        transaction
            .commit()
            .await
            .context("commit SQLite source write reopen")?;
        Ok(())
    }

    pub(crate) async fn rollback_source_write_fence(
        &self,
        fenced_generation: u64,
        restored_generation: u64,
    ) -> Result<()> {
        if restored_generation == 0 || fenced_generation <= restored_generation {
            bail!("source authority generations must roll back to a prior non-zero generation");
        }
        let mut transaction = self.pool.begin().await?;
        let state = self.load_capture_state_tx(&mut transaction).await?;
        if state.authority_generation != fenced_generation || !state.source_writes_fenced {
            bail!("SQLite source write fence changed before rollback");
        }
        let state_table = quote_identifier(
            &self.capture_state_table_name,
            "SQLite migration capture state table",
        )?;
        let query = format!(
            "UPDATE {state_table}
                SET authority_generation = ?, source_writes_fenced = 0
              WHERE migration_id = ?
                AND source_identity = ?
                AND authority_generation = ?
                AND source_writes_fenced = 1"
        );
        let result = sqlx::query(&query)
            .bind(i64::try_from(restored_generation).context("authority generation overflow")?)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .bind(i64::try_from(fenced_generation).context("authority generation overflow")?)
            .execute(&mut *transaction)
            .await
            .context("rollback SQLite source write fence")?;
        if result.rows_affected() != 1 {
            bail!("SQLite source write authority changed during rollback");
        }
        transaction
            .commit()
            .await
            .context("commit SQLite source write fence rollback")?;
        Ok(())
    }

    fn spec(&self, table_name: &str) -> Result<&MigrationTableSpec> {
        self.specs
            .get(table_name)
            .ok_or_else(|| anyhow!("SQLite migration table is not mapped: {table_name}"))
    }

    async fn read_changes(
        &self,
        after: i64,
        through: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>> {
        if after < 0 || through < after {
            bail!("invalid SQLite migration change range");
        }
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query_limit = limits
            .max_rows
            .checked_add(1)
            .ok_or_else(|| anyhow!("SQLite migration change row limit overflowed"))?;
        let query_limit_i64 =
            i64::try_from(query_limit).context("SQLite migration change limit overflowed")?;
        let query = format!(
            "SELECT migration_id, source_identity, source_version, table_name, stable_key,
                    previous_key, kind, payload, payload_digest
               FROM {capture_table}
              WHERE migration_id = ?
                AND source_identity = ?
                AND source_version > ? AND source_version <= ?
              ORDER BY source_version ASC, capture_id ASC LIMIT ?"
        );
        let mut transaction = self.pool.begin().await?;
        let capture_state = self.load_capture_state_tx(&mut transaction).await?;
        if capture_state.continuity_lost {
            bail!(
                "SQLite migration capture continuity has been lost; a new capture generation and baseline are required"
            );
        }
        let rows = sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .bind(after)
            .bind(through)
            .bind(query_limit_i64)
            .fetch_all(&mut *transaction)
            .await
            .context("read SQLite migration change capture")?;
        let changes = rows
            .into_iter()
            .map(|row| decode_sqlite_change(row, &self.migration_id, &self.source_identity))
            .collect::<Result<Vec<_>>>()?;
        // Fetch one sentinel row beyond the row budget. This lets the shared selector distinguish
        // a complete boundary from a partial source transaction without ever returning a partial
        // source version. A source transaction that itself exceeds either budget is rejected.
        let selected = select_change_batch(&changes, limits)?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration change read")?;
        Ok(selected)
    }

    async fn native_table_bounds(
        &self,
        spec: &NativeTableSpec,
    ) -> Result<(Option<String>, Option<String>)> {
        let table = quote_identifier(&spec.owner.table, "native source table")?;
        let key = sqlite_native_key_expression(spec, "source");
        let query = format!(
            "SELECT MIN({key}) AS lower_bound, MAX({key}) AS upper_bound FROM {table} AS source"
        );
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&query)
            .fetch_one(&mut *transaction)
            .await
            .context("read native SQLite source bounds")?;
        let bounds = (
            row.try_get::<Option<String>, _>("lower_bound")?,
            row.try_get::<Option<String>, _>("upper_bound")?,
        );
        transaction
            .commit()
            .await
            .context("commit native SQLite source bounds read")?;
        Ok(bounds)
    }

    async fn native_scan_range_page(
        &self,
        spec: &NativeTableSpec,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourceRangePage> {
        limits.validate()?;
        let table = quote_identifier(spec.table_name(), "native source table")?;
        let columns = native_sqlite_select_columns(spec, "source");
        let key = sqlite_native_key_expression(spec, "source");
        let mut predicates = Vec::new();
        let mut binds = Vec::new();
        if let Some(start) = cursor.or(lower_bound) {
            predicates.push(format!(
                "{key} {} ?",
                if cursor.is_some() { ">" } else { ">=" }
            ));
            binds.push(start);
        }
        if let Some(upper_bound) = upper_bound {
            predicates.push(format!("{key} <= ?"));
            binds.push(upper_bound);
        }
        let where_clause = if predicates.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", predicates.join(" AND "))
        };
        let query = format!(
            "SELECT {columns}, {key} AS __cvm_stable_key
               FROM {table} AS source{where_clause}
              ORDER BY {key} ASC LIMIT ?"
        );
        let mut sql_query = sqlx::query(&query);
        for bind in binds {
            sql_query = sql_query.bind(bind);
        }
        sql_query = sql_query.bind(
            i64::try_from(
                limits
                    .max_rows
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("native SQLite page row limit overflowed"))?,
            )
            .context("native SQLite page row limit overflowed")?,
        );
        let mut transaction = self.pool.begin().await?;
        let raw_rows = sql_query
            .fetch_all(&mut *transaction)
            .await
            .context("read native SQLite migration page")?;
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        let mut complete = true;
        for row in raw_rows {
            if rows.len() >= limits.max_rows {
                complete = false;
                break;
            }
            let native_row = NativeRow::from_sqlite_row(spec, &row)?;
            let payload = native_row.encode_payload()?;
            if payload.len() > limits.max_bytes && rows.is_empty() {
                bail!("native SQLite row exceeds the copy byte budget");
            }
            let next_bytes = bytes
                .checked_add(payload.len())
                .ok_or_else(|| anyhow!("native SQLite page byte count overflowed"))?;
            if next_bytes > limits.max_bytes {
                complete = false;
                break;
            }
            bytes = next_bytes;
            rows.push(VersionedRow {
                stable_key: row.try_get("__cvm_stable_key")?,
                source_version: native_source_version(spec, &native_row, 0)?,
                payload,
            });
        }
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let watermark_query = format!(
            "SELECT COALESCE(MAX(source_version), 0) FROM {capture_table}
              WHERE migration_id = ? AND source_identity = ?"
        );
        let source_watermark = sqlx::query_scalar(&watermark_query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .fetch_one(&mut *transaction)
            .await
            .context("read native SQLite migration watermark")?;
        transaction
            .commit()
            .await
            .context("commit native SQLite migration page")?;
        Ok(SourceRangePage {
            source_watermark,
            page: SourcePage {
                next_cursor: rows.last().map(|row| row.stable_key.clone()),
                complete,
                rows,
            },
        })
    }
}

fn sqlite_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sqlite_native_capture_payload(spec: &NativeTableSpec, alias: &str) -> String {
    let fields = spec
        .columns
        .iter()
        .map(|column| {
            let quoted = format!("{alias}.{}", native_quote_identifier(&column.name));
            let value = format!(
                "CASE WHEN typeof({quoted}) = 'blob' \
                 THEN json_object('__cvm_blob_hex', hex({quoted})) \
                 ELSE {quoted} END"
            );
            format!("{}, {value}", sqlite_string_literal(&column.name))
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("json_object({fields})")
}

#[expect(
    clippy::too_many_arguments,
    reason = "The trigger body mirrors the native capture contract fields."
)]
fn native_capture_trigger_body(
    capture_table: &str,
    state_table: &str,
    migration_id: &str,
    source_identity: &str,
    table_name: &str,
    stable_key: &str,
    previous_key: &str,
    kind: &str,
    payload: &str,
) -> String {
    let migration_id = sqlite_string_literal(migration_id);
    let source_identity = sqlite_string_literal(source_identity);
    let table_name = sqlite_string_literal(table_name);
    let kind = sqlite_string_literal(kind);
    let payload_length = if payload == "NULL" {
        "0".to_string()
    } else {
        format!("length(CAST({payload} AS BLOB))")
    };
    format!(
        "BEGIN
            INSERT INTO {capture_table}
                (migration_id, source_identity, source_version, table_name,
                 stable_key, previous_key, kind, payload, payload_digest)
            VALUES (
                {migration_id}, {source_identity},
                (SELECT next_version + 1 FROM {state_table}
                  WHERE migration_id = {migration_id}
                    AND source_identity = {source_identity}),
                {table_name}, {stable_key}, {previous_key}, {kind},
                {payload}, ''
            );
            UPDATE {state_table}
               SET next_version = next_version + 1,
                   captured_bytes = captured_bytes + {payload_length},
                   capture_capacity_held = CASE
                       WHEN captured_bytes + {payload_length} > max_capture_bytes THEN 1
                       ELSE capture_capacity_held
                   END
             WHERE migration_id = {migration_id}
               AND source_identity = {source_identity};
        END"
    )
}

fn native_quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn sqlite_native_key_expression(spec: &NativeTableSpec, qualifier: &str) -> String {
    spec.stable_key_columns()
        .iter()
        .map(|column| {
            let column = format!(
                "CAST({qualifier}.{} AS TEXT)",
                native_quote_identifier(column)
            );
            format!("printf('%08d:%s', length(COALESCE({column}, '')), COALESCE({column}, ''))")
        })
        .collect::<Vec<_>>()
        .join(" || char(31) || ")
}

fn postgres_native_key_expression(
    spec: &NativeTableSpec,
    qualifier: &str,
    contract: Option<&NativeTargetContract>,
) -> String {
    contract
        .map(|contract| contract.stable_key_columns.as_slice())
        .unwrap_or_else(|| spec.stable_key_columns())
        .iter()
        .map(|column| {
            let column = format!(
                "CAST({qualifier}.{} AS TEXT)",
                native_quote_identifier(column)
            );
            format!(
                "lpad(length(COALESCE({column}, ''))::TEXT, 8, '0') || ':' || COALESCE({column}, '')"
            )
        })
        .collect::<Vec<_>>()
        .join(" || chr(31) || ")
}

fn native_sqlite_select_columns(spec: &NativeTableSpec, qualifier: &str) -> String {
    spec.columns
        .iter()
        .map(|column| {
            let quoted = native_quote_identifier(&column.name);
            format!("{qualifier}.{quoted} AS {quoted}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeTargetColumn {
    source_column: String,
    target_column: String,
    source_type: NativeColumnType,
    target_type: NativeColumnType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeTargetContract {
    table_name: String,
    columns: Vec<NativeTargetColumn>,
    stable_key_columns: Vec<String>,
}

impl NativeTargetContract {
    pub(crate) fn for_spec(spec: &NativeTableSpec) -> Result<Self> {
        let source_column = |name: &str| {
            spec.columns
                .iter()
                .find(|column| column.name == name)
                .ok_or_else(|| anyhow!("native target contract is missing source column {name}"))
        };
        let mut columns = Vec::new();
        if spec.table_name() == "archive_batches" {
            let add = |columns: &mut Vec<NativeTargetColumn>,
                       source: &str,
                       target: &str,
                       target_type: NativeColumnType| {
                let source_definition = source_column(source)?;
                columns.push(NativeTargetColumn {
                    source_column: source.to_owned(),
                    target_column: target.to_owned(),
                    source_type: source_definition.postgres_type,
                    target_type,
                });
                Ok::<(), anyhow::Error>(())
            };
            // The archive owner deliberately exposes a different PostgreSQL identity. Keep every
            // conversion in this contract so INSERT, replay, and verification cannot silently
            // fall back to SQLite names. The native integer `part` column remains owned by
            // ArchiveManifest writes; migrated source values use the nullable text `part_key`
            // column so segment identifiers and legacy NULLs remain lossless.
            add(
                &mut columns,
                "id",
                "archive_batch_id",
                NativeColumnType::BigInt,
            )?;
            for name in [
                "dataset",
                "month_key",
                "day_key",
                "file_path",
                "sha256",
                "row_count",
                "status",
                "layout",
                "codec",
                "writer_version",
                "cleanup_state",
                "cleanup_source_safe_start_date",
                "replacement_staged_path",
                "superseded_by",
                "coverage_start_at",
                "coverage_end_at",
                "coverage_start_epoch",
                "coverage_end_epoch",
                "archive_expires_at",
                "summary_source_kind",
                "created_at",
            ] {
                add(&mut columns, name, name, source_column(name)?.postgres_type)?;
            }
            add(&mut columns, "part_key", "part_key", NativeColumnType::Text)?;
            add(
                &mut columns,
                "sha256",
                "source_identity_sha256",
                NativeColumnType::Text,
            )?;
            // These target columns are newer than the SQLite source schema. The archive digest is
            // the only durable identity available in the source row, so use it explicitly rather
            // than relying on a target default that could hide a missing mapping.
            for name in [
                "upstream_activity_manifest_refreshed_at",
                "historical_rollups_materialized_at",
            ] {
                if spec.columns.iter().any(|column| column.name == name) {
                    add(&mut columns, name, name, source_column(name)?.postgres_type)?;
                }
            }
            let source_names = spec
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<BTreeSet<_>>();
            let mapped_names = columns
                .iter()
                .map(|column| column.source_column.as_str())
                .collect::<BTreeSet<_>>();
            let unmapped = source_names
                .difference(&mapped_names)
                .copied()
                .collect::<Vec<_>>();
            if !unmapped.is_empty() {
                bail!(
                    "archive_batches native target contract does not map source columns: {}",
                    unmapped.join(", ")
                );
            }
            Ok(Self {
                table_name: spec.table_name().to_owned(),
                columns,
                stable_key_columns: vec!["archive_batch_id".to_owned()],
            })
        } else {
            for column in &spec.columns {
                columns.push(NativeTargetColumn {
                    source_column: column.name.clone(),
                    target_column: column.name.clone(),
                    source_type: column.postgres_type,
                    target_type: column.postgres_type,
                });
            }
            Ok(Self {
                table_name: spec.table_name().to_owned(),
                columns,
                stable_key_columns: spec.stable_key_columns().to_vec(),
            })
        }
    }

    fn column_for_source(&self, source_column: &str) -> Result<&NativeTargetColumn> {
        self.columns
            .iter()
            .find(|column| column.source_column == source_column)
            .ok_or_else(|| {
                anyhow!("native target contract is missing source column {source_column}")
            })
    }

    pub(crate) fn target_column_for_source(&self, source_column: &str) -> Result<&str> {
        Ok(&self.column_for_source(source_column)?.target_column)
    }

    pub(crate) fn target_columns(&self) -> impl Iterator<Item = &str> {
        self.columns
            .iter()
            .map(|column| column.target_column.as_str())
    }
}

fn native_postgres_select_columns(
    spec: &NativeTableSpec,
    qualifier: &str,
    contract: &NativeTargetContract,
) -> Result<String> {
    spec.columns
        .iter()
        .map(|column| {
            let mapping = contract.column_for_source(&column.name)?;
            let target = format!(
                "{qualifier}.{}",
                native_quote_identifier(&mapping.target_column)
            );
            let expression = if mapping.source_type == mapping.target_type {
                target
            } else {
                format!("CAST({target} AS {})", mapping.source_type.postgres_name())
            };
            Ok(format!(
                "{expression} AS {}",
                native_quote_identifier(&column.name)
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(|columns| columns.join(", "))
}

fn native_source_version(spec: &NativeTableSpec, row: &NativeRow, fallback: i64) -> Result<i64> {
    let Some(column) = spec
        .source_watermark
        .business_source_version_column
        .as_deref()
    else {
        return Ok(fallback);
    };
    match row.values().iter().find(|(name, _)| name == column) {
        Some((_, NativeValue::BigInt(value))) if *value >= 0 => Ok(*value),
        Some((_, NativeValue::Null)) => Ok(fallback),
        Some((_, value)) => bail!("native source watermark column {column} has value {value:?}"),
        None => bail!("native source watermark column {column} is missing"),
    }
}

fn bind_pg_native_value<'q>(
    query: Query<'q, Postgres, PgArguments>,
    column_type: NativeColumnType,
    value: &NativeValue,
) -> Result<Query<'q, Postgres, PgArguments>> {
    match (column_type, value) {
        (NativeColumnType::BigInt, NativeValue::Null) => Ok(query.bind(Option::<i64>::None)),
        (NativeColumnType::BigInt, NativeValue::BigInt(value)) => Ok(query.bind(*value)),
        (NativeColumnType::DoublePrecision, NativeValue::Null) => {
            Ok(query.bind(Option::<f64>::None))
        }
        (NativeColumnType::DoublePrecision, NativeValue::Double(value)) => Ok(query.bind(*value)),
        (NativeColumnType::Boolean, NativeValue::Null) => Ok(query.bind(Option::<bool>::None)),
        (NativeColumnType::Boolean, NativeValue::Boolean(value)) => Ok(query.bind(*value)),
        (NativeColumnType::Text, NativeValue::Null) => Ok(query.bind(Option::<String>::None)),
        (NativeColumnType::Text, NativeValue::Text(value)) => Ok(query.bind(value.clone())),
        (NativeColumnType::Bytea, NativeValue::Null) => Ok(query.bind(Option::<Vec<u8>>::None)),
        (NativeColumnType::Bytea, NativeValue::Bytes(value)) => Ok(query.bind(value.clone())),
        (_, value) => bail!("native row value {value:?} does not match its PostgreSQL type"),
    }
}

fn bind_pg_native_target_value<'q>(
    query: Query<'q, Postgres, PgArguments>,
    column_type: NativeColumnType,
    value: &NativeValue,
) -> Result<Query<'q, Postgres, PgArguments>> {
    match (column_type, value) {
        (NativeColumnType::BigInt, NativeValue::Text(value)) => Ok(query.bind(
            value
                .parse::<i64>()
                .with_context(|| format!("native target BIGINT is not numeric: {value}"))?,
        )),
        (NativeColumnType::Text, NativeValue::BigInt(value)) => Ok(query.bind(value.to_string())),
        (NativeColumnType::Text, NativeValue::Boolean(value)) => {
            Ok(query.bind(if *value { "1" } else { "0" }.to_owned()))
        }
        (NativeColumnType::Text, NativeValue::Double(value)) => Ok(query.bind(value.to_string())),
        (NativeColumnType::DoublePrecision, NativeValue::BigInt(value)) => {
            Ok(query.bind(*value as f64))
        }
        (NativeColumnType::DoublePrecision, NativeValue::Text(value)) => {
            Ok(query.bind(value.parse::<f64>().with_context(|| {
                format!("native target DOUBLE PRECISION is not numeric: {value}")
            })?))
        }
        _ => bind_pg_native_value(query, column_type, value),
    }
}

async fn upsert_native_row_tx(
    transaction: &mut Transaction<'_, Postgres>,
    spec: &NativeTableSpec,
    row: &VersionedRow,
) -> Result<()> {
    let native_row = NativeRow::decode_payload(spec, &row.payload)?;
    let contract = NativeTargetContract::for_spec(spec)?;
    let table = native_quote_identifier(&contract.table_name);
    let columns = contract
        .columns
        .iter()
        .map(|column| native_quote_identifier(&column.target_column))
        .collect::<Vec<_>>();
    let key_columns = contract
        .stable_key_columns
        .iter()
        .map(|column| native_quote_identifier(column))
        .collect::<Vec<_>>();
    let updates = contract
        .columns
        .iter()
        .filter(|column| !contract.stable_key_columns.contains(&column.target_column))
        .map(|column| {
            let quoted = native_quote_identifier(&column.target_column);
            format!("{quoted} = EXCLUDED.{quoted}")
        })
        .collect::<Vec<_>>();
    let conflict = if updates.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!("DO UPDATE SET {}", updates.join(", "))
    };
    let placeholders = (1..=columns.len())
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let query_sql = format!(
        "INSERT INTO {table} ({}) VALUES ({placeholders}) ON CONFLICT ({}) {conflict}",
        columns.join(", "),
        key_columns.join(", ")
    );
    let mut query = sqlx::query(&query_sql);
    for mapping in &contract.columns {
        let value = native_row
            .values()
            .iter()
            .find(|(column, _)| column == &mapping.source_column)
            .map(|(_, value)| value)
            .ok_or_else(|| {
                anyhow!(
                    "native row contains unknown source column {}",
                    mapping.source_column
                )
            })?;
        query = bind_pg_native_target_value(query, mapping.target_type, value)?;
    }
    query
        .execute(&mut **transaction)
        .await
        .with_context(|| format!("upsert native PostgreSQL row {}", row.stable_key))?;
    Ok(())
}

async fn delete_native_row_tx(
    transaction: &mut Transaction<'_, Postgres>,
    spec: &NativeTableSpec,
    stable_key: &str,
    _source_version: i64,
) -> Result<()> {
    let contract = NativeTargetContract::for_spec(spec)?;
    let table = native_quote_identifier(&contract.table_name);
    let key = postgres_native_key_expression(spec, "target", Some(&contract));
    let query = format!("DELETE FROM {table} AS target WHERE {key} = $1");
    sqlx::query(&query)
        .bind(stable_key)
        .execute(&mut **transaction)
        .await
        .context("delete native PostgreSQL row")?;
    Ok(())
}

async fn native_postgres_scan_range_page(
    pool: &PgPool,
    spec: &NativeTableSpec,
    cursor: Option<&str>,
    lower_bound: Option<&str>,
    upper_bound: Option<&str>,
    limits: CopyLimits,
) -> Result<SourcePage> {
    limits.validate()?;
    let contract = NativeTargetContract::for_spec(spec)?;
    let table = native_quote_identifier(&contract.table_name);
    let columns = native_postgres_select_columns(spec, "target", &contract)?;
    let key = postgres_native_key_expression(spec, "target", Some(&contract));
    let mut predicates = Vec::new();
    let mut binds = Vec::new();
    if let Some(start) = cursor.or(lower_bound) {
        predicates.push(format!(
            "{key} {} ${}",
            if cursor.is_some() { ">" } else { ">=" },
            binds.len() + 1
        ));
        binds.push(start);
    }
    if let Some(upper_bound) = upper_bound {
        predicates.push(format!("{key} <= ${}", binds.len() + 1));
        binds.push(upper_bound);
    }
    let where_clause = if predicates.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", predicates.join(" AND "))
    };
    let limit_parameter = binds.len() + 1;
    let query = format!(
        "SELECT {columns}, {key} AS __cvm_stable_key
           FROM {table} AS target{where_clause}
          ORDER BY {key} ASC LIMIT ${limit_parameter}"
    );
    let mut sql_query = sqlx::query(&query);
    for bind in binds {
        sql_query = sql_query.bind(bind);
    }
    sql_query = sql_query.bind(
        i64::try_from(
            limits
                .max_rows
                .checked_add(1)
                .ok_or_else(|| anyhow!("native PostgreSQL page row limit overflowed"))?,
        )
        .context("native PostgreSQL page row limit overflowed")?,
    );
    let mut transaction = pool.begin().await?;
    let raw_rows = sql_query
        .fetch_all(&mut *transaction)
        .await
        .context("read native PostgreSQL migration page")?;
    let mut rows = Vec::new();
    let mut bytes = 0_usize;
    let mut complete = true;
    for row in raw_rows {
        if rows.len() >= limits.max_rows {
            complete = false;
            break;
        }
        let native_row = NativeRow::from_postgres_row(spec, &row)?;
        let payload = native_row.encode_payload()?;
        if payload.len() > limits.max_bytes && rows.is_empty() {
            bail!("native PostgreSQL row exceeds the copy byte budget");
        }
        let next_bytes = bytes
            .checked_add(payload.len())
            .ok_or_else(|| anyhow!("native PostgreSQL page byte count overflowed"))?;
        if next_bytes > limits.max_bytes {
            complete = false;
            break;
        }
        bytes = next_bytes;
        rows.push(VersionedRow {
            stable_key: row.try_get("__cvm_stable_key")?,
            source_version: native_source_version(spec, &native_row, 0)?,
            payload,
        });
    }
    transaction
        .commit()
        .await
        .context("commit native PostgreSQL migration page")?;
    Ok(SourcePage {
        next_cursor: rows.last().map(|row| row.stable_key.clone()),
        complete,
        rows,
    })
}

async fn native_postgres_table_bounds(
    pool: &PgPool,
    spec: &NativeTableSpec,
) -> Result<(Option<String>, Option<String>)> {
    let contract = NativeTargetContract::for_spec(spec)?;
    let table = native_quote_identifier(&contract.table_name);
    let key = postgres_native_key_expression(spec, "target", Some(&contract));
    let query = format!(
        "SELECT MIN({key}) AS lower_bound, MAX({key}) AS upper_bound FROM {table} AS target"
    );
    let row = sqlx::query(&query)
        .fetch_one(pool)
        .await
        .context("read native PostgreSQL target bounds")?;
    Ok((
        row.try_get::<Option<String>, _>("lower_bound")?,
        row.try_get::<Option<String>, _>("upper_bound")?,
    ))
}

fn decode_sqlite_change(
    row: SqliteRow,
    migration_id: &str,
    source_identity: &str,
) -> Result<CapturedChange> {
    let kind = parse_change_kind(row.try_get::<String, _>("kind")?.as_str())?;
    let payload: Option<Vec<u8>> = row.try_get("payload")?;
    let mut change = CapturedChange {
        migration_id: row.try_get("migration_id")?,
        source_identity: row.try_get("source_identity")?,
        source_version: row.try_get("source_version")?,
        table_name: row.try_get("table_name")?,
        stable_key: row.try_get("stable_key")?,
        previous_key: row.try_get("previous_key")?,
        kind,
        payload: payload.clone(),
        payload_digest: row.try_get("payload_digest")?,
    };
    // Native SQLite triggers cannot compute SHA-256 inside SQL. An empty digest is therefore a
    // deliberate trigger marker; compute the same digest as application-side capture before
    // validating the row.
    if change.payload_digest.is_empty() {
        change = CapturedChange::from_input(
            &change.migration_id,
            &change.source_identity,
            change.source_version,
            ChangeInput {
                table_name: change.table_name.clone(),
                stable_key: change.stable_key.clone(),
                previous_key: change.previous_key.clone(),
                kind,
                payload,
            },
        )?;
    }
    change.validate_for(migration_id, source_identity)?;
    Ok(change)
}

fn change_kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Insert => "insert",
        ChangeKind::Update => "update",
        ChangeKind::Delete => "delete",
        ChangeKind::Replace => "replace",
    }
}

fn parse_change_kind(kind: &str) -> Result<ChangeKind> {
    match kind {
        "insert" => Ok(ChangeKind::Insert),
        "update" => Ok(ChangeKind::Update),
        "delete" => Ok(ChangeKind::Delete),
        "replace" => Ok(ChangeKind::Replace),
        _ => bail!("unknown migration change kind: {kind}"),
    }
}

#[async_trait]
impl MigrationSource for SqliteMigrationSource {
    async fn capture_watermark(&self) -> Result<i64> {
        Ok(self.capture_checkpoint().await?.next_version)
    }

    async fn capture_checkpoint(&self) -> Result<CaptureCheckpoint> {
        SqliteMigrationSource::capture_checkpoint(self).await
    }

    async fn mark_baseline_complete(&self, capture_generation: u64, through: i64) -> Result<()> {
        SqliteMigrationSource::mark_baseline_complete(self, capture_generation, through).await
    }

    async fn fence_source_writes(
        &self,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<()> {
        SqliteMigrationSource::fence_source_writes(self, expected_generation, next_generation).await
    }

    async fn reopen_source_writes(&self, expected_generation: u64) -> Result<()> {
        SqliteMigrationSource::reopen_source_writes(self, expected_generation).await
    }

    async fn rollback_source_write_fence(
        &self,
        fenced_generation: u64,
        restored_generation: u64,
    ) -> Result<()> {
        SqliteMigrationSource::rollback_source_write_fence(
            self,
            fenced_generation,
            restored_generation,
        )
        .await
    }

    async fn module_tables(&self, module: StorageModule) -> Result<Vec<String>> {
        let tables = if self
            .specs
            .values()
            .filter(|spec| spec.module == module)
            .all(|spec| spec.native.is_some())
        {
            ordered_native_module_table_names(&self.specs, module)?
        } else {
            module_table_names(&self.specs, module)
        };
        if tables.is_empty() {
            bail!("SQLite migration source has no mapped tables for {module:?}");
        }
        Ok(tables)
    }

    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)> {
        let spec = self.spec(table_name)?;
        if let Some(native) = spec.native.as_ref() {
            return self.native_table_bounds(native).await;
        }
        let table = quote_identifier(&spec.source_table_name, "source migration table")?;
        let key = quote_identifier(&spec.source_key_column, "source stable-key column")?;
        let query = format!(
            "SELECT MIN({key} COLLATE BINARY) AS lower_bound,
                        MAX({key} COLLATE BINARY) AS upper_bound
                   FROM {table}"
        );
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&query)
            .fetch_one(&mut *transaction)
            .await
            .context("read SQLite source bounds")?;
        let bounds = (
            row.try_get::<Option<String>, _>("lower_bound")?,
            row.try_get::<Option<String>, _>("upper_bound")?,
        );
        transaction
            .commit()
            .await
            .context("commit SQLite migration source bounds read")?;
        Ok(bounds)
    }

    async fn scan_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage> {
        Ok(self
            .scan_range_page(table_name, cursor, None, upper_bound, limits)
            .await?
            .page)
    }

    async fn scan_range_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourceRangePage> {
        if limits.max_rows == 0 || limits.max_bytes == 0 {
            bail!("SQLite migration page limits must be positive");
        }
        let spec = self.spec(table_name)?;
        if let Some(native) = spec.native.as_ref() {
            return self
                .native_scan_range_page(native, cursor, lower_bound, upper_bound, limits)
                .await;
        }
        let table = quote_identifier(&spec.source_table_name, "source migration table")?;
        let key = quote_identifier(&spec.source_key_column, "source stable-key column")?;
        let version = quote_identifier(&spec.source_version_column, "source version column")?;
        let payload = quote_identifier(&spec.source_payload_column, "source payload column")?;
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let mut transaction = self.pool.begin().await?;
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        let mut complete = true;
        if let Some(upper_bound) = upper_bound {
            let start = cursor.or(lower_bound).unwrap_or_default();
            let comparison = if cursor.is_some() { ">" } else { ">=" };
            let query = format!(
                "SELECT {key} AS stable_key, {version} AS source_version,
                        COALESCE(length({payload}), 0) AS payload_bytes
                   FROM {table}
                  WHERE {key} COLLATE BINARY {comparison} ?
                    AND {key} COLLATE BINARY <= ?
                  ORDER BY {key} COLLATE BINARY ASC LIMIT ?"
            );
            let raw_rows = sqlx::query(&query)
                .bind(start)
                .bind(upper_bound)
                .bind(
                    i64::try_from(
                        limits
                            .max_rows
                            .checked_add(1)
                            .ok_or_else(|| anyhow!("SQLite page row limit overflowed"))?,
                    )
                    .context("SQLite page row limit overflowed")?,
                )
                .fetch_all(&mut *transaction)
                .await
                .context("read SQLite migration page metadata")?;
            for row in raw_rows {
                if rows.len() >= limits.max_rows {
                    complete = false;
                    break;
                }
                let stable_key: String = row.try_get("stable_key")?;
                let source_version: i64 = row.try_get("source_version")?;
                let payload_bytes = usize::try_from(row.try_get::<i64, _>("payload_bytes")?)
                    .context("SQLite migration payload length overflowed")?;
                if rows.is_empty() && payload_bytes > limits.max_bytes {
                    bail!("SQLite migration row exceeds the copy byte budget");
                }
                let next_bytes = bytes
                    .checked_add(payload_bytes)
                    .ok_or_else(|| anyhow!("SQLite migration page byte count overflowed"))?;
                if next_bytes > limits.max_bytes {
                    complete = false;
                    break;
                }
                let payload_query = format!(
                    "SELECT {payload} AS payload FROM {table}
                          WHERE {key} COLLATE BINARY = ?"
                );
                let payload_row = sqlx::query(&payload_query)
                    .bind(&stable_key)
                    .fetch_one(&mut *transaction)
                    .await
                    .context("read SQLite migration page payload")?;
                let payload = payload_row
                    .try_get::<Option<Vec<u8>>, _>("payload")?
                    .unwrap_or_default();
                if payload.len() != payload_bytes {
                    bail!("SQLite migration page payload changed within its read transaction");
                }
                bytes = next_bytes;
                rows.push(VersionedRow {
                    stable_key,
                    source_version,
                    payload,
                });
            }
        }
        let watermark_query = format!(
            "SELECT COALESCE(MAX(source_version), 0)
               FROM {capture_table}
              WHERE migration_id = ? AND source_identity = ?"
        );
        let source_watermark = sqlx::query_scalar(&watermark_query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .fetch_one(&mut *transaction)
            .await
            .context("read SQLite migration page watermark")?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration page")?;
        Ok(SourceRangePage {
            source_watermark,
            page: SourcePage {
                complete,
                next_cursor: rows.last().map(|row| row.stable_key.clone()),
                rows,
            },
        })
    }

    async fn rows_in_range_snapshot(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<SourceRangeSnapshot> {
        let mut cursor = None;
        let mut rows = Vec::new();
        let mut source_watermark = None;
        loop {
            let range_page = self
                .scan_range_page(
                    table_name,
                    cursor.as_deref(),
                    Some(&range.first_key),
                    Some(&range.last_key),
                    CopyLimits::default(),
                )
                .await?;
            if let Some(previous_watermark) = source_watermark
                && previous_watermark != range_page.source_watermark
            {
                bail!("SQLite verification range changed between bounded pages");
            }
            source_watermark = Some(range_page.source_watermark);
            let next_cursor = range_page.page.next_cursor.clone();
            rows.extend(range_page.page.rows);
            if range_page.page.complete {
                break;
            }
            if next_cursor.as_deref() == cursor.as_deref() {
                bail!("SQLite verification page did not advance");
            }
            cursor = next_cursor;
        }
        Ok(SourceRangeSnapshot {
            source_watermark: source_watermark.unwrap_or_default(),
            rows,
        })
    }

    async fn changes_after(
        &self,
        after: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>> {
        self.read_changes(after, i64::MAX, limits).await
    }

    async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>> {
        self.read_changes(after, through, limits).await
    }

    async fn release_capture_through(&self, through: i64) -> Result<()> {
        SqliteMigrationSource::release_capture_through(self, through).await?;
        Ok(())
    }

    fn migration_id(&self) -> &str {
        &self.migration_id
    }

    fn source_identity(&self) -> &str {
        &self.source_identity
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PostgresMigrationTarget {
    pool: PgPool,
    metadata: PostgresMigrationMetadataTables,
    specs: Arc<BTreeMap<String, MigrationTableSpec>>,
    native_catalog: Option<NativeMigrationCatalog>,
}

#[derive(Debug, Clone)]
struct PostgresMigrationMetadataTables {
    plans: String,
    checkpoints: String,
    chunk_proofs: String,
    coverage_checkpoints: String,
    coverage_proofs: String,
}

impl PostgresMigrationMetadataTables {
    fn new(prefix: &str) -> Result<Self> {
        validate_identifier(prefix, "PostgreSQL migration metadata prefix")?;
        Ok(Self {
            plans: format!("{prefix}_plans"),
            checkpoints: format!("{prefix}_checkpoints"),
            chunk_proofs: format!("{prefix}_chunk_proofs"),
            coverage_checkpoints: format!("{prefix}_coverage_checkpoints"),
            coverage_proofs: format!("{prefix}_coverage_proofs"),
        })
    }
}

impl PostgresMigrationTarget {
    pub(crate) fn new(
        pool: PgPool,
        metadata_prefix: impl AsRef<str>,
        specs: Vec<MigrationTableSpec>,
    ) -> Result<Self> {
        Ok(Self {
            pool,
            metadata: PostgresMigrationMetadataTables::new(metadata_prefix.as_ref())?,
            specs: Arc::new(spec_map(specs)?),
            native_catalog: None,
        })
    }

    pub(crate) fn new_native(
        pool: PgPool,
        metadata_prefix: impl AsRef<str>,
        catalog: NativeMigrationCatalog,
        module: StorageModule,
    ) -> Result<Self> {
        let specs = catalog
            .tables
            .iter()
            .filter(|spec| {
                spec.owner.module
                    == match module {
                        StorageModule::TrafficArchive => ModuleId::TrafficArchive,
                        StorageModule::Analytics => ModuleId::Analytics,
                        StorageModule::AccountRouting => ModuleId::AccountRouting,
                        StorageModule::TaskOperations => ModuleId::TaskOperations,
                        StorageModule::Performance => ModuleId::Performance,
                    }
            })
            .map(|spec| MigrationTableSpec::native(module, spec.clone()))
            .collect::<Result<Vec<_>>>()?;
        if specs.is_empty() {
            bail!("native migration catalog has no target tables for {module:?}");
        }
        Ok(Self {
            pool,
            metadata: PostgresMigrationMetadataTables::new(metadata_prefix.as_ref())?,
            specs: Arc::new(spec_map(specs)?),
            native_catalog: Some(catalog),
        })
    }

    async fn ensure_target_table_schema(&self, spec: &MigrationTableSpec) -> Result<()> {
        if spec.native.is_some() {
            return Ok(());
        }
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let version = quote_identifier(&spec.target_version_column, "target version column")?;
        let payload = quote_identifier(&spec.target_payload_column, "target payload column")?;
        let query = format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                {key} {} PRIMARY KEY,
                {version} {} NOT NULL,
                {payload} {} NOT NULL
            )",
            spec.target_schema.key_type.postgres_name(),
            spec.target_schema.version_type.postgres_name(),
            spec.target_schema.payload_type.postgres_name(),
        );
        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .with_context(|| {
                format!(
                    "create PostgreSQL migration target table {}",
                    spec.target_table_name
                )
            })?;

        let rows = sqlx::query(
            "SELECT column_name, data_type, is_nullable
               FROM information_schema.columns
              WHERE table_schema = current_schema() AND table_name = $1",
        )
        .bind(&spec.target_table_name)
        .fetch_all(&self.pool)
        .await
        .with_context(|| {
            format!(
                "inspect PostgreSQL migration target table {}",
                spec.target_table_name
            )
        })?;
        for (column, expected_type) in [
            (&spec.target_key_column, spec.target_schema.key_type),
            (&spec.target_version_column, spec.target_schema.version_type),
            (&spec.target_payload_column, spec.target_schema.payload_type),
        ] {
            let row = rows
                .iter()
                .find(|row| row.try_get::<String, _>("column_name").ok().as_deref() == Some(column))
                .ok_or_else(|| {
                    anyhow!(
                        "PostgreSQL migration target table {} is missing column {column}",
                        spec.target_table_name
                    )
                })?;
            let actual_type: String = row.try_get("data_type")?;
            let nullable: String = row.try_get("is_nullable")?;
            if actual_type != expected_type.information_schema_name() || nullable != "NO" {
                bail!(
                    "PostgreSQL migration target column {}.{} has incompatible schema",
                    spec.target_table_name,
                    column
                );
            }
        }
        Ok(())
    }

    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        if let Some(catalog) = self.native_catalog.clone() {
            super::native::PostgresNativeSchemaAdapter::new(self.pool.clone(), catalog)
                .ensure_schema()
                .await?;
        }
        let mut target_tables = BTreeSet::new();
        for spec in self.specs.values() {
            if target_tables.insert(spec.target_table_name.clone()) {
                self.ensure_target_table_schema(spec).await?;
            }
        }
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let chunk_proofs = quote_identifier(
            &self.metadata.chunk_proofs,
            "PostgreSQL migration chunk proofs table",
        )?;
        let coverage_checkpoints = quote_identifier(
            &self.metadata.coverage_checkpoints,
            "PostgreSQL migration coverage checkpoints table",
        )?;
        let coverage_proofs = quote_identifier(
            &self.metadata.coverage_proofs,
            "PostgreSQL migration coverage proofs table",
        )?;
        for query in [
            format!(
                "CREATE TABLE IF NOT EXISTS {plans} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
                    installation_id TEXT NOT NULL,
                    source_identity TEXT NOT NULL,
                    target_identity TEXT NOT NULL,
                    scan_lower_bound TEXT,
                    scan_upper_bound TEXT,
                    capture_start BIGINT NOT NULL,
                    PRIMARY KEY (migration_id, table_name)
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {checkpoints} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
                    installation_id TEXT NOT NULL,
                    source_identity TEXT NOT NULL,
                    target_identity TEXT NOT NULL,
                    cursor TEXT,
                    scan_upper_bound TEXT,
                    capture_start BIGINT NOT NULL,
                    captured_through BIGINT NOT NULL,
                    applied_through BIGINT NOT NULL,
                    copied_rows BIGINT NOT NULL,
                    copied_bytes BIGINT NOT NULL,
                    complete BOOLEAN NOT NULL,
                    PRIMARY KEY (migration_id, table_name)
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {chunk_proofs} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
                    installation_id TEXT NOT NULL,
                    source_identity TEXT NOT NULL,
                    target_identity TEXT NOT NULL,
                    first_key TEXT NOT NULL,
                    last_key TEXT NOT NULL,
                    source_watermark BIGINT NOT NULL,
                    row_count BIGINT NOT NULL,
                    byte_count BIGINT NOT NULL,
                    digest TEXT NOT NULL,
                    PRIMARY KEY (migration_id, table_name, first_key, last_key)
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {coverage_checkpoints} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
                    installation_id TEXT NOT NULL,
                    source_identity TEXT NOT NULL,
                    target_identity TEXT NOT NULL,
                    lower_bound TEXT,
                    upper_bound TEXT,
                    cursor TEXT,
                    source_watermark BIGINT NOT NULL,
                    source_row_count BIGINT NOT NULL,
                    target_row_count BIGINT NOT NULL,
                    source_byte_count BIGINT NOT NULL,
                    target_byte_count BIGINT NOT NULL,
                    source_digest TEXT NOT NULL,
                    target_digest TEXT NOT NULL,
                    complete BOOLEAN NOT NULL,
                    PRIMARY KEY (migration_id, table_name)
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {coverage_proofs} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
                    installation_id TEXT NOT NULL,
                    source_identity TEXT NOT NULL,
                    target_identity TEXT NOT NULL,
                    source_watermark BIGINT NOT NULL,
                    range_first_key TEXT,
                    range_last_key TEXT,
                    source_lower_bound TEXT,
                    source_upper_bound TEXT,
                    target_lower_bound TEXT,
                    target_upper_bound TEXT,
                    source_row_count BIGINT NOT NULL,
                    target_row_count BIGINT NOT NULL,
                    source_byte_count BIGINT NOT NULL,
                    target_byte_count BIGINT NOT NULL,
                    source_digest TEXT NOT NULL,
                    target_digest TEXT NOT NULL,
                    PRIMARY KEY (migration_id, table_name)
                )"
            ),
        ] {
            sqlx::query(&query)
                .execute(&self.pool)
                .await
                .context("create PostgreSQL migration metadata table")?;
        }
        for (table, column) in [
            (&plans, "installation_id"),
            (&plans, "source_identity"),
            (&plans, "target_identity"),
            (&checkpoints, "installation_id"),
            (&checkpoints, "source_identity"),
            (&checkpoints, "target_identity"),
            (&chunk_proofs, "installation_id"),
            (&chunk_proofs, "source_identity"),
            (&chunk_proofs, "target_identity"),
            (&coverage_checkpoints, "installation_id"),
            (&coverage_checkpoints, "source_identity"),
            (&coverage_checkpoints, "target_identity"),
            (&coverage_proofs, "installation_id"),
            (&coverage_proofs, "source_identity"),
            (&coverage_proofs, "target_identity"),
        ] {
            let column = quote_identifier(column, "PostgreSQL migration identity column")?;
            let query = format!(
                "ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {column} TEXT NOT NULL DEFAULT ''"
            );
            sqlx::query(&query)
                .execute(&self.pool)
                .await
                .context("upgrade PostgreSQL migration metadata identity columns")?;
        }
        Ok(())
    }

    fn spec(&self, table_name: &str) -> Result<&MigrationTableSpec> {
        self.specs
            .get(table_name)
            .ok_or_else(|| anyhow!("PostgreSQL migration table is not mapped: {table_name}"))
    }

    async fn native_scan_range_page(
        &self,
        spec: &NativeTableSpec,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage> {
        native_postgres_scan_range_page(&self.pool, spec, cursor, lower_bound, upper_bound, limits)
            .await
    }

    async fn native_table_bounds(
        &self,
        spec: &NativeTableSpec,
    ) -> Result<(Option<String>, Option<String>)> {
        native_postgres_table_bounds(&self.pool, spec).await
    }

    async fn load_plan_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCopyPlan>> {
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    scan_lower_bound, scan_upper_bound, capture_start
               FROM {plans}
              WHERE migration_id = $1 AND table_name = $2 FOR UPDATE"
        );
        sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&mut **transaction)
            .await
            .context("load PostgreSQL migration plan in transaction")?
            .map(decode_plan)
            .transpose()
    }

    async fn load_checkpoint_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<EngineCheckpoint>> {
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    cursor, scan_upper_bound, capture_start,
                    captured_through, applied_through, copied_rows, copied_bytes, complete
               FROM {checkpoints}
              WHERE migration_id = $1 AND table_name = $2 FOR UPDATE"
        );
        sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&mut **transaction)
            .await
            .context("load PostgreSQL migration checkpoint in transaction")?
            .map(decode_checkpoint)
            .transpose()
    }

    async fn persist_checkpoint_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        checkpoint: &EngineCheckpoint,
    ) -> Result<()> {
        checkpoint.validate()?;
        if let Some(existing) = self
            .load_checkpoint_tx(
                transaction,
                &checkpoint.migration_id,
                &checkpoint.table_name,
            )
            .await?
            && (existing.installation_id != checkpoint.installation_id
                || existing.source_identity != checkpoint.source_identity
                || existing.target_identity != checkpoint.target_identity)
        {
            bail!("PostgreSQL migration checkpoint belongs to another identity");
        }
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let query = format!(
            "INSERT INTO {checkpoints}
             (migration_id, table_name, installation_id, source_identity, target_identity,
              cursor, scan_upper_bound, capture_start,
              captured_through, applied_through, copied_rows, copied_bytes, complete)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (migration_id, table_name) DO UPDATE SET
                installation_id = EXCLUDED.installation_id,
                source_identity = EXCLUDED.source_identity,
                target_identity = EXCLUDED.target_identity,
                cursor = EXCLUDED.cursor, scan_upper_bound = EXCLUDED.scan_upper_bound,
                capture_start = EXCLUDED.capture_start, captured_through = EXCLUDED.captured_through,
                applied_through = EXCLUDED.applied_through, copied_rows = EXCLUDED.copied_rows,
                copied_bytes = EXCLUDED.copied_bytes, complete = EXCLUDED.complete"
        );
        sqlx::query(&query)
            .bind(&checkpoint.migration_id)
            .bind(&checkpoint.table_name)
            .bind(&checkpoint.installation_id)
            .bind(&checkpoint.source_identity)
            .bind(&checkpoint.target_identity)
            .bind(&checkpoint.cursor)
            .bind(&checkpoint.scan_upper_bound)
            .bind(checkpoint.capture_start)
            .bind(checkpoint.captured_through)
            .bind(checkpoint.applied_through)
            .bind(i64_from_u64(checkpoint.copied_rows, "copied rows")?)
            .bind(i64_from_u64(checkpoint.copied_bytes, "copied bytes")?)
            .bind(checkpoint.complete)
            .execute(&mut **transaction)
            .await
            .context("persist PostgreSQL migration checkpoint")?;
        Ok(())
    }

    async fn upsert_row_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        spec: &MigrationTableSpec,
        row: &VersionedRow,
    ) -> Result<()> {
        if let Some(native) = spec.native.as_ref() {
            return upsert_native_row_tx(transaction, native, row).await;
        }
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let version = quote_identifier(&spec.target_version_column, "target version column")?;
        let payload = quote_identifier(&spec.target_payload_column, "target payload column")?;
        let query = format!(
            "INSERT INTO {table} AS target ({key}, {version}, {payload})
             VALUES ($1, $2, $3)
             ON CONFLICT ({key}) DO UPDATE SET
                {version} = EXCLUDED.{version}, {payload} = EXCLUDED.{payload}
             WHERE target.{version} <= EXCLUDED.{version}"
        );
        sqlx::query(&query)
            .bind(&row.stable_key)
            .bind(row.source_version)
            .bind(&row.payload)
            .execute(&mut **transaction)
            .await
            .with_context(|| format!("upsert PostgreSQL migration row {}", row.stable_key))?;
        Ok(())
    }

    async fn delete_row_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        spec: &MigrationTableSpec,
        stable_key: &str,
        source_version: i64,
    ) -> Result<()> {
        if let Some(native) = spec.native.as_ref() {
            return delete_native_row_tx(transaction, native, stable_key, source_version).await;
        }
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let version = quote_identifier(&spec.target_version_column, "target version column")?;
        let query = format!("DELETE FROM {table} WHERE {key} = $1 AND {version} <= $2");
        sqlx::query(&query)
            .bind(stable_key)
            .bind(source_version)
            .execute(&mut **transaction)
            .await
            .context("delete PostgreSQL migration row")?;
        Ok(())
    }

    async fn apply_change_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        change: &CapturedChange,
    ) -> Result<()> {
        let spec = self.spec(&change.table_name)?;
        match change.kind {
            ChangeKind::Insert | ChangeKind::Update => {
                self.upsert_row_tx(
                    transaction,
                    spec,
                    &VersionedRow {
                        stable_key: change.stable_key.clone(),
                        source_version: change.source_version,
                        payload: change.payload.clone().unwrap_or_default(),
                    },
                )
                .await?;
            }
            ChangeKind::Delete => {
                self.delete_row_tx(transaction, spec, &change.stable_key, change.source_version)
                    .await?;
            }
            ChangeKind::Replace => {
                if let Some(previous_key) = change.previous_key.as_deref() {
                    self.delete_row_tx(transaction, spec, previous_key, change.source_version)
                        .await?;
                }
                self.upsert_row_tx(
                    transaction,
                    spec,
                    &VersionedRow {
                        stable_key: change.stable_key.clone(),
                        source_version: change.source_version,
                        payload: change.payload.clone().unwrap_or_default(),
                    },
                )
                .await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MigrationTarget for PostgresMigrationTarget {
    async fn known_tables(&self, migration_id: &str) -> Result<Vec<String>> {
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let query =
            format!("SELECT table_name FROM {plans} WHERE migration_id = $1 ORDER BY table_name");
        Ok(sqlx::query_scalar(&query)
            .bind(migration_id)
            .fetch_all(&self.pool)
            .await
            .context("list PostgreSQL migration plans")?)
    }

    async fn load_plan(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCopyPlan>> {
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    scan_lower_bound, scan_upper_bound, capture_start
               FROM {plans} WHERE migration_id = $1 AND table_name = $2"
        );
        sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&self.pool)
            .await
            .context("load PostgreSQL migration plan")?
            .map(decode_plan)
            .transpose()
    }

    async fn save_plan(&self, plan: &TableCopyPlan) -> Result<()> {
        self.spec(&plan.table_name)?;
        plan.validate()?;
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let mut transaction = self.pool.begin().await?;
        let query = format!(
            "INSERT INTO {plans}
             (migration_id, table_name, installation_id, source_identity, target_identity,
              scan_lower_bound, scan_upper_bound, capture_start)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (migration_id, table_name) DO NOTHING"
        );
        sqlx::query(&query)
            .bind(&plan.migration_id)
            .bind(&plan.table_name)
            .bind(&plan.installation_id)
            .bind(&plan.source_identity)
            .bind(&plan.target_identity)
            .bind(&plan.scan_lower_bound)
            .bind(&plan.scan_upper_bound)
            .bind(plan.capture_start)
            .execute(&mut *transaction)
            .await
            .context("persist PostgreSQL migration plan")?;
        let stored = self
            .load_plan_tx(&mut transaction, &plan.migration_id, &plan.table_name)
            .await?
            .ok_or_else(|| anyhow!("PostgreSQL migration plan disappeared during save"))?;
        if stored != *plan {
            bail!("PostgreSQL migration plan conflicts with existing durable plan");
        }
        let checkpoint_query = format!(
            "INSERT INTO {checkpoints}
             (migration_id, table_name, installation_id, source_identity, target_identity,
              cursor, scan_upper_bound, capture_start,
              captured_through, applied_through, copied_rows, copied_bytes, complete)
             VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, $7, $7, 0, 0, FALSE)
             ON CONFLICT (migration_id, table_name) DO NOTHING"
        );
        sqlx::query(&checkpoint_query)
            .bind(&plan.migration_id)
            .bind(&plan.table_name)
            .bind(&plan.installation_id)
            .bind(&plan.source_identity)
            .bind(&plan.target_identity)
            .bind(&plan.scan_upper_bound)
            .bind(plan.capture_start)
            .execute(&mut *transaction)
            .await
            .context("initialize PostgreSQL migration checkpoint")?;
        transaction
            .commit()
            .await
            .context("commit PostgreSQL migration plan")?;
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<EngineCheckpoint>> {
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    cursor, scan_upper_bound, capture_start,
                    captured_through, applied_through, copied_rows, copied_bytes, complete
               FROM {checkpoints} WHERE migration_id = $1 AND table_name = $2"
        );
        sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&self.pool)
            .await
            .context("load PostgreSQL migration checkpoint")?
            .map(decode_checkpoint)
            .transpose()
    }

    async fn apply_baseline_page(
        &self,
        plan: &TableCopyPlan,
        page: &SourcePage,
        checkpoint: &EngineCheckpoint,
    ) -> Result<()> {
        checkpoint.validate()?;
        if checkpoint.migration_id != plan.migration_id
            || checkpoint.table_name != plan.table_name
            || checkpoint.installation_id != plan.installation_id
            || checkpoint.source_identity != plan.source_identity
            || checkpoint.target_identity != plan.target_identity
        {
            bail!("PostgreSQL baseline checkpoint does not match its plan");
        }
        let mut transaction = self.pool.begin().await?;
        let existing = self
            .load_checkpoint_tx(&mut transaction, &plan.migration_id, &plan.table_name)
            .await?
            .ok_or_else(|| anyhow!("PostgreSQL migration checkpoint is missing"))?;
        if existing.installation_id != checkpoint.installation_id
            || existing.source_identity != checkpoint.source_identity
            || existing.target_identity != checkpoint.target_identity
        {
            bail!("PostgreSQL baseline checkpoint belongs to another identity");
        }
        validate_checkpoint_progress(Some(&existing), checkpoint)?;
        let spec = self.spec(&plan.table_name)?;
        for row in &page.rows {
            self.upsert_row_tx(&mut transaction, spec, row).await?;
        }
        self.persist_checkpoint_tx(&mut transaction, checkpoint)
            .await?;
        transaction
            .commit()
            .await
            .context("commit PostgreSQL migration baseline page")?;
        Ok(())
    }

    async fn apply_change_batch(
        &self,
        migration_id: &str,
        source_identity: &str,
        changes: &[CapturedChange],
        checkpoint: &EngineCheckpoint,
    ) -> Result<()> {
        checkpoint.validate()?;
        if checkpoint.migration_id != migration_id {
            bail!("PostgreSQL replay checkpoint does not match its migration");
        }
        let mut transaction = self.pool.begin().await?;
        let existing = self
            .load_checkpoint_tx(&mut transaction, migration_id, &checkpoint.table_name)
            .await?
            .ok_or_else(|| anyhow!("PostgreSQL migration checkpoint is missing"))?;
        if existing.installation_id != checkpoint.installation_id
            || existing.source_identity != checkpoint.source_identity
            || existing.target_identity != checkpoint.target_identity
        {
            bail!("PostgreSQL replay checkpoint belongs to another identity");
        }
        validate_checkpoint_progress(Some(&existing), checkpoint)?;
        for change in changes {
            change.validate_for(migration_id, source_identity)?;
            if change.table_name != checkpoint.table_name
                || change.source_version > checkpoint.captured_through
            {
                bail!("PostgreSQL replay change does not match its checkpoint");
            }
            self.apply_change_tx(&mut transaction, change).await?;
        }
        self.persist_checkpoint_tx(&mut transaction, checkpoint)
            .await?;
        transaction
            .commit()
            .await
            .context("commit PostgreSQL migration replay batch")?;
        Ok(())
    }

    async fn scan_range_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage> {
        limits.validate()?;
        let spec = self.spec(table_name)?;
        if let Some(native) = spec.native.as_ref() {
            return self
                .native_scan_range_page(native, cursor, lower_bound, upper_bound, limits)
                .await;
        }
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let version = quote_identifier(&spec.target_version_column, "target version column")?;
        let payload = quote_identifier(&spec.target_payload_column, "target payload column")?;
        let mut transaction = self.pool.begin().await?;
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        let mut complete = true;
        if let Some(upper_bound) = upper_bound {
            let start = cursor.or(lower_bound).unwrap_or_default();
            let comparison = if cursor.is_some() { ">" } else { ">=" };
            let query = format!(
                "SELECT {key} AS stable_key, {version} AS source_version,
                        COALESCE(octet_length({payload}), 0)::BIGINT AS payload_bytes
                   FROM {table}
                  WHERE {key} COLLATE \"C\" {comparison} $1
                    AND {key} COLLATE \"C\" <= $2
                  ORDER BY {key} COLLATE \"C\" ASC LIMIT $3"
            );
            let raw_rows = sqlx::query(&query)
                .bind(start)
                .bind(upper_bound)
                .bind(
                    i64::try_from(
                        limits
                            .max_rows
                            .checked_add(1)
                            .ok_or_else(|| anyhow!("PostgreSQL page row limit overflowed"))?,
                    )
                    .context("PostgreSQL page row limit overflowed")?,
                )
                .fetch_all(&mut *transaction)
                .await
                .context("read PostgreSQL migration page metadata")?;
            for row in raw_rows {
                if rows.len() >= limits.max_rows {
                    complete = false;
                    break;
                }
                let stable_key: String = row.try_get("stable_key")?;
                let source_version: i64 = row.try_get("source_version")?;
                let payload_bytes = usize::try_from(row.try_get::<i64, _>("payload_bytes")?)
                    .context("PostgreSQL migration payload length overflowed")?;
                if rows.is_empty() && payload_bytes > limits.max_bytes {
                    bail!("PostgreSQL migration row exceeds the coverage byte budget");
                }
                let next_bytes = bytes
                    .checked_add(payload_bytes)
                    .ok_or_else(|| anyhow!("PostgreSQL migration page byte count overflowed"))?;
                if next_bytes > limits.max_bytes {
                    complete = false;
                    break;
                }
                let payload_query = format!(
                    "SELECT {payload} AS payload FROM {table}
                      WHERE {key} COLLATE \"C\" = $1"
                );
                let payload_row = sqlx::query(&payload_query)
                    .bind(&stable_key)
                    .fetch_one(&mut *transaction)
                    .await
                    .context("read PostgreSQL migration page payload")?;
                let payload = payload_row
                    .try_get::<Option<Vec<u8>>, _>("payload")?
                    .unwrap_or_default();
                if payload.len() != payload_bytes {
                    bail!("PostgreSQL migration page payload changed within its read transaction");
                }
                bytes = next_bytes;
                rows.push(VersionedRow {
                    stable_key,
                    source_version,
                    payload,
                });
            }
        }
        transaction
            .commit()
            .await
            .context("commit PostgreSQL migration page")?;
        Ok(SourcePage {
            complete,
            next_cursor: rows.last().map(|row| row.stable_key.clone()),
            rows,
        })
    }

    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>> {
        let mut cursor = None;
        let mut rows = Vec::new();
        loop {
            let page = self
                .scan_range_page(
                    table_name,
                    cursor.as_deref(),
                    Some(&range.first_key),
                    Some(&range.last_key),
                    CopyLimits::default(),
                )
                .await?;
            let next_cursor = page.next_cursor.clone();
            rows.extend(page.rows);
            if page.complete {
                break;
            }
            if next_cursor.as_deref() == cursor.as_deref() {
                bail!("PostgreSQL migration verification page did not advance");
            }
            cursor = next_cursor;
        }
        Ok(rows)
    }

    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)> {
        let spec = self.spec(table_name)?;
        if let Some(native) = spec.native.as_ref() {
            return self.native_table_bounds(native).await;
        }
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let query = format!(
            "SELECT MIN({key} COLLATE \"C\") AS lower_bound,
                        MAX({key} COLLATE \"C\") AS upper_bound
                   FROM {table}"
        );
        let row = sqlx::query(&query)
            .fetch_one(&self.pool)
            .await
            .context("read PostgreSQL target table bounds")?;
        Ok((row.try_get("lower_bound")?, row.try_get("upper_bound")?))
    }

    async fn save_chunk_proof(&self, proof: &ChunkProof) -> Result<()> {
        proof.validate()?;
        let chunk_proofs = quote_identifier(
            &self.metadata.chunk_proofs,
            "PostgreSQL migration chunk proofs table",
        )?;
        let query = format!(
            "INSERT INTO {chunk_proofs}
             (migration_id, table_name, installation_id, source_identity, target_identity,
              first_key, last_key, source_watermark, row_count, byte_count, digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT (migration_id, table_name, first_key, last_key) DO UPDATE SET
                installation_id = EXCLUDED.installation_id,
                source_identity = EXCLUDED.source_identity,
                target_identity = EXCLUDED.target_identity,
                source_watermark = EXCLUDED.source_watermark,
                row_count = EXCLUDED.row_count, byte_count = EXCLUDED.byte_count,
                digest = EXCLUDED.digest
             WHERE {chunk_proofs}.installation_id = EXCLUDED.installation_id
               AND {chunk_proofs}.source_identity = EXCLUDED.source_identity
               AND {chunk_proofs}.target_identity = EXCLUDED.target_identity"
        );
        let result = sqlx::query(&query)
            .bind(&proof.migration_id)
            .bind(&proof.table_name)
            .bind(&proof.installation_id)
            .bind(&proof.source_identity)
            .bind(&proof.target_identity)
            .bind(&proof.range.first_key)
            .bind(&proof.range.last_key)
            .bind(proof.source_watermark)
            .bind(i64_from_u64(proof.row_count, "chunk proof row count")?)
            .bind(i64_from_u64(proof.byte_count, "chunk proof byte count")?)
            .bind(&proof.digest)
            .execute(&self.pool)
            .await
            .context("persist PostgreSQL migration chunk proof")?;
        if result.rows_affected() != 1 {
            bail!("PostgreSQL migration chunk proof belongs to another identity");
        }
        Ok(())
    }

    async fn verified_chunks(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Vec<ChunkProof>> {
        let chunk_proofs = quote_identifier(
            &self.metadata.chunk_proofs,
            "PostgreSQL migration chunk proofs table",
        )?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    first_key, last_key, source_watermark,
                    row_count, byte_count, digest
               FROM {chunk_proofs}
              WHERE migration_id = $1 AND table_name = $2
              ORDER BY first_key, last_key"
        );
        let rows = sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_all(&self.pool)
            .await
            .context("load PostgreSQL migration chunk proofs")?;
        rows.into_iter()
            .map(|row| {
                let proof = ChunkProof {
                    migration_id: row.try_get("migration_id")?,
                    table_name: row.try_get("table_name")?,
                    installation_id: row.try_get("installation_id")?,
                    source_identity: row.try_get("source_identity")?,
                    target_identity: row.try_get("target_identity")?,
                    range: ChunkRange::new(
                        row.try_get::<String, _>("first_key")?,
                        row.try_get::<String, _>("last_key")?,
                    )?,
                    source_watermark: row.try_get("source_watermark")?,
                    row_count: u64_from_i64(row.try_get("row_count")?, "chunk proof row count")?,
                    byte_count: u64_from_i64(row.try_get("byte_count")?, "chunk proof byte count")?,
                    digest: row.try_get("digest")?,
                };
                proof.validate()?;
                Ok(proof)
            })
            .collect()
    }

    async fn save_coverage_checkpoint(&self, checkpoint: &CoverageCheckpoint) -> Result<()> {
        checkpoint.validate()?;
        let coverage_checkpoints = quote_identifier(
            &self.metadata.coverage_checkpoints,
            "PostgreSQL migration coverage checkpoints table",
        )?;
        let query = format!(
            "INSERT INTO {coverage_checkpoints}
             (migration_id, table_name, installation_id, source_identity, target_identity,
              lower_bound, upper_bound, cursor, source_watermark,
              source_row_count, target_row_count, source_byte_count, target_byte_count,
              source_digest, target_digest, complete)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (migration_id, table_name) DO UPDATE SET
                installation_id = EXCLUDED.installation_id,
                source_identity = EXCLUDED.source_identity,
                target_identity = EXCLUDED.target_identity,
                lower_bound = EXCLUDED.lower_bound, upper_bound = EXCLUDED.upper_bound,
                cursor = EXCLUDED.cursor, source_watermark = EXCLUDED.source_watermark,
                source_row_count = EXCLUDED.source_row_count,
                target_row_count = EXCLUDED.target_row_count,
                source_byte_count = EXCLUDED.source_byte_count,
                target_byte_count = EXCLUDED.target_byte_count,
                source_digest = EXCLUDED.source_digest, target_digest = EXCLUDED.target_digest,
                complete = EXCLUDED.complete
             WHERE {coverage_checkpoints}.installation_id = EXCLUDED.installation_id
               AND {coverage_checkpoints}.source_identity = EXCLUDED.source_identity
               AND {coverage_checkpoints}.target_identity = EXCLUDED.target_identity"
        );
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(&query)
            .bind(&checkpoint.migration_id)
            .bind(&checkpoint.table_name)
            .bind(&checkpoint.installation_id)
            .bind(&checkpoint.source_identity)
            .bind(&checkpoint.target_identity)
            .bind(&checkpoint.lower_bound)
            .bind(&checkpoint.upper_bound)
            .bind(&checkpoint.cursor)
            .bind(checkpoint.source_watermark)
            .bind(i64_from_u64(
                checkpoint.source_row_count,
                "coverage checkpoint source row count",
            )?)
            .bind(i64_from_u64(
                checkpoint.target_row_count,
                "coverage checkpoint target row count",
            )?)
            .bind(i64_from_u64(
                checkpoint.source_byte_count,
                "coverage checkpoint source byte count",
            )?)
            .bind(i64_from_u64(
                checkpoint.target_byte_count,
                "coverage checkpoint target byte count",
            )?)
            .bind(&checkpoint.source_digest)
            .bind(&checkpoint.target_digest)
            .bind(checkpoint.complete)
            .execute(&mut *transaction)
            .await
            .context("persist PostgreSQL migration coverage checkpoint")?;
        if result.rows_affected() != 1 {
            bail!("PostgreSQL migration coverage checkpoint belongs to another identity");
        }
        transaction
            .commit()
            .await
            .context("commit PostgreSQL migration coverage checkpoint")?;
        Ok(())
    }

    async fn load_coverage_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<CoverageCheckpoint>> {
        let coverage_checkpoints = quote_identifier(
            &self.metadata.coverage_checkpoints,
            "PostgreSQL migration coverage checkpoints table",
        )?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    lower_bound, upper_bound, cursor,
                    source_watermark, source_row_count, target_row_count,
                    source_byte_count, target_byte_count, source_digest, target_digest, complete
               FROM {coverage_checkpoints}
              WHERE migration_id = $1 AND table_name = $2"
        );
        sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&self.pool)
            .await
            .context("load PostgreSQL migration coverage checkpoint")?
            .map(decode_coverage_checkpoint)
            .transpose()
    }

    async fn save_table_coverage_proof(&self, proof: &TableCoverageProof) -> Result<()> {
        proof.validate()?;
        let coverage_proofs = quote_identifier(
            &self.metadata.coverage_proofs,
            "PostgreSQL migration coverage proofs table",
        )?;
        let (range_first_key, range_last_key) = proof
            .range
            .as_ref()
            .map(|range| {
                (
                    Some(range.first_key.as_str()),
                    Some(range.last_key.as_str()),
                )
            })
            .unwrap_or((None, None));
        let query = format!(
            "INSERT INTO {coverage_proofs}
             (migration_id, table_name, installation_id, source_identity, target_identity,
              source_watermark, range_first_key, range_last_key,
              source_lower_bound, source_upper_bound, target_lower_bound, target_upper_bound,
              source_row_count, target_row_count, source_byte_count, target_byte_count,
              source_digest, target_digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
             ON CONFLICT (migration_id, table_name) DO UPDATE SET
                installation_id = EXCLUDED.installation_id,
                source_identity = EXCLUDED.source_identity,
                target_identity = EXCLUDED.target_identity,
                source_watermark = EXCLUDED.source_watermark,
                range_first_key = EXCLUDED.range_first_key,
                range_last_key = EXCLUDED.range_last_key,
                source_lower_bound = EXCLUDED.source_lower_bound,
                source_upper_bound = EXCLUDED.source_upper_bound,
                target_lower_bound = EXCLUDED.target_lower_bound,
                target_upper_bound = EXCLUDED.target_upper_bound,
                source_row_count = EXCLUDED.source_row_count,
                target_row_count = EXCLUDED.target_row_count,
                source_byte_count = EXCLUDED.source_byte_count,
                target_byte_count = EXCLUDED.target_byte_count,
                source_digest = EXCLUDED.source_digest,
                target_digest = EXCLUDED.target_digest
             WHERE {coverage_proofs}.installation_id = EXCLUDED.installation_id
               AND {coverage_proofs}.source_identity = EXCLUDED.source_identity
               AND {coverage_proofs}.target_identity = EXCLUDED.target_identity"
        );
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(&query)
            .bind(&proof.migration_id)
            .bind(&proof.table_name)
            .bind(&proof.installation_id)
            .bind(&proof.source_identity)
            .bind(&proof.target_identity)
            .bind(proof.source_watermark)
            .bind(range_first_key)
            .bind(range_last_key)
            .bind(&proof.source_lower_bound)
            .bind(&proof.source_upper_bound)
            .bind(&proof.target_lower_bound)
            .bind(&proof.target_upper_bound)
            .bind(i64_from_u64(
                proof.source_row_count,
                "coverage source row count",
            )?)
            .bind(i64_from_u64(
                proof.target_row_count,
                "coverage target row count",
            )?)
            .bind(i64_from_u64(
                proof.source_byte_count,
                "coverage source byte count",
            )?)
            .bind(i64_from_u64(
                proof.target_byte_count,
                "coverage target byte count",
            )?)
            .bind(&proof.source_digest)
            .bind(&proof.target_digest)
            .execute(&mut *transaction)
            .await
            .context("persist PostgreSQL migration coverage proof")?;
        if result.rows_affected() != 1 {
            bail!("PostgreSQL migration coverage proof belongs to another identity");
        }
        transaction
            .commit()
            .await
            .context("commit PostgreSQL migration coverage proof")?;
        Ok(())
    }

    async fn load_table_coverage_proof(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCoverageProof>> {
        let coverage_proofs = quote_identifier(
            &self.metadata.coverage_proofs,
            "PostgreSQL migration coverage proofs table",
        )?;
        let query = format!(
            "SELECT migration_id, table_name, installation_id, source_identity, target_identity,
                    source_watermark, range_first_key, range_last_key,
                    source_lower_bound, source_upper_bound, target_lower_bound, target_upper_bound,
                    source_row_count, target_row_count, source_byte_count, target_byte_count,
                    source_digest, target_digest
               FROM {coverage_proofs}
              WHERE migration_id = $1 AND table_name = $2"
        );
        let Some(row) = sqlx::query(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&self.pool)
            .await
            .context("load PostgreSQL migration coverage proof")?
        else {
            return Ok(None);
        };
        let range = match (
            row.try_get::<Option<String>, _>("range_first_key")?,
            row.try_get::<Option<String>, _>("range_last_key")?,
        ) {
            (None, None) => None,
            (Some(first), Some(last)) => Some(ChunkRange::new(first, last)?),
            _ => bail!("PostgreSQL migration coverage proof has an incomplete range"),
        };
        let proof = TableCoverageProof {
            migration_id: row.try_get("migration_id")?,
            table_name: row.try_get("table_name")?,
            installation_id: row.try_get("installation_id")?,
            source_identity: row.try_get("source_identity")?,
            target_identity: row.try_get("target_identity")?,
            source_watermark: row.try_get("source_watermark")?,
            range,
            source_lower_bound: row.try_get("source_lower_bound")?,
            source_upper_bound: row.try_get("source_upper_bound")?,
            target_lower_bound: row.try_get("target_lower_bound")?,
            target_upper_bound: row.try_get("target_upper_bound")?,
            source_row_count: u64_from_i64(
                row.try_get("source_row_count")?,
                "coverage source row count",
            )?,
            target_row_count: u64_from_i64(
                row.try_get("target_row_count")?,
                "coverage target row count",
            )?,
            source_byte_count: u64_from_i64(
                row.try_get("source_byte_count")?,
                "coverage source byte count",
            )?,
            target_byte_count: u64_from_i64(
                row.try_get("target_byte_count")?,
                "coverage target byte count",
            )?,
            source_digest: row.try_get("source_digest")?,
            target_digest: row.try_get("target_digest")?,
        };
        proof.validate()?;
        Ok(Some(proof))
    }
}

fn decode_plan(row: sqlx::postgres::PgRow) -> Result<TableCopyPlan> {
    let plan = TableCopyPlan {
        migration_id: row.try_get("migration_id")?,
        table_name: row.try_get("table_name")?,
        installation_id: row.try_get("installation_id")?,
        source_identity: row.try_get("source_identity")?,
        target_identity: row.try_get("target_identity")?,
        scan_lower_bound: row.try_get("scan_lower_bound")?,
        scan_upper_bound: row.try_get("scan_upper_bound")?,
        capture_start: row.try_get("capture_start")?,
    };
    plan.validate()?;
    Ok(plan)
}

fn decode_checkpoint(row: sqlx::postgres::PgRow) -> Result<EngineCheckpoint> {
    let checkpoint = EngineCheckpoint {
        migration_id: row.try_get("migration_id")?,
        table_name: row.try_get("table_name")?,
        installation_id: row.try_get("installation_id")?,
        source_identity: row.try_get("source_identity")?,
        target_identity: row.try_get("target_identity")?,
        cursor: row.try_get("cursor")?,
        scan_upper_bound: row.try_get("scan_upper_bound")?,
        capture_start: row.try_get("capture_start")?,
        captured_through: row.try_get("captured_through")?,
        applied_through: row.try_get("applied_through")?,
        copied_rows: u64_from_i64(row.try_get("copied_rows")?, "copied rows")?,
        copied_bytes: u64_from_i64(row.try_get("copied_bytes")?, "copied bytes")?,
        complete: row.try_get("complete")?,
    };
    checkpoint.validate()?;
    Ok(checkpoint)
}

fn decode_coverage_checkpoint(row: sqlx::postgres::PgRow) -> Result<CoverageCheckpoint> {
    let checkpoint = CoverageCheckpoint {
        migration_id: row.try_get("migration_id")?,
        table_name: row.try_get("table_name")?,
        installation_id: row.try_get("installation_id")?,
        source_identity: row.try_get("source_identity")?,
        target_identity: row.try_get("target_identity")?,
        lower_bound: row.try_get("lower_bound")?,
        upper_bound: row.try_get("upper_bound")?,
        cursor: row.try_get("cursor")?,
        source_watermark: row.try_get("source_watermark")?,
        source_row_count: u64_from_i64(
            row.try_get("source_row_count")?,
            "coverage checkpoint source row count",
        )?,
        target_row_count: u64_from_i64(
            row.try_get("target_row_count")?,
            "coverage checkpoint target row count",
        )?,
        source_byte_count: u64_from_i64(
            row.try_get("source_byte_count")?,
            "coverage checkpoint source byte count",
        )?,
        target_byte_count: u64_from_i64(
            row.try_get("target_byte_count")?,
            "coverage checkpoint target byte count",
        )?,
        source_digest: row.try_get("source_digest")?,
        target_digest: row.try_get("target_digest")?,
        complete: row.try_get("complete")?,
    };
    checkpoint.validate()?;
    Ok(checkpoint)
}

fn i64_from_u64(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{label} exceeds PostgreSQL BIGINT"))
}

fn u64_from_i64(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value).with_context(|| format!("{label} is negative"))
}
