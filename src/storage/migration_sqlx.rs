//! SQLx-backed migration adapters.
//!
//! This module owns bounded database mechanics. Business modules provide one
//! `MigrationTableSpec` per logical table, including the physical columns used for
//! stable identity, source version, and serialized payload. That keeps traffic and
//! archive schema ownership in pg03/pg04 instead of guessing it here.

use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Row, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow};

use super::{
    catalog::{ModuleCatalog, ModuleId},
    core::StorageModule,
    migration::{
        CapturedChange, ChangeInput, ChangeKind, ChunkProof, ChunkRange, CopyLimits,
        EngineCheckpoint, MigrationSource, MigrationTarget, SourcePage, SourceRangeSnapshot,
        TableCopyPlan, TableCoverageProof, VersionedRow, validate_checkpoint_progress,
    },
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
    specs: Arc<BTreeMap<String, MigrationTableSpec>>,
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
        Ok(Self {
            pool,
            migration_id,
            source_identity,
            capture_table_name,
            specs: Arc::new(spec_map(specs)?),
        })
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
        Ok(())
    }

    pub(crate) async fn append_captured_change(&self, change: &CapturedChange) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        self.append_captured_change_tx(&mut transaction, change)
            .await?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration change capture")?;
        Ok(())
    }

    pub(crate) async fn append_change(
        &self,
        source_version: i64,
        input: ChangeInput,
    ) -> Result<CapturedChange> {
        let change = CapturedChange::from_input(
            &self.migration_id,
            &self.source_identity,
            source_version,
            input,
        )?;
        self.append_captured_change(&change).await?;
        Ok(change)
    }

    pub(crate) async fn append_captured_change_tx(
        &self,
        transaction: &mut Transaction<'_, Sqlite>,
        change: &CapturedChange,
    ) -> Result<()> {
        change.validate_for(&self.migration_id, &self.source_identity)?;
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
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        if after < 0 || through < after || limit == 0 {
            bail!("invalid SQLite migration change range");
        }
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query = format!(
            "SELECT migration_id, source_identity, source_version, table_name, stable_key,
                    previous_key, kind, payload, payload_digest
               FROM {capture_table}
              WHERE migration_id = ?
                AND source_version > ? AND source_version <= ?
              ORDER BY source_version ASC, capture_id ASC LIMIT ?"
        );
        let rows = sqlx::query(&query)
            .bind(&self.migration_id)
            .bind(after)
            .bind(through)
            .bind(i64::try_from(limit).context("SQLite migration change limit overflowed")?)
            .fetch_all(&self.pool)
            .await
            .context("read SQLite migration change capture")?;
        let initial_changes = rows
            .into_iter()
            .map(|row| decode_sqlite_change(row, &self.migration_id, &self.source_identity))
            .collect::<Result<Vec<_>>>()?;
        let Some(boundary_version) = initial_changes.last().map(|change| change.source_version)
        else {
            return Ok(Vec::new());
        };
        if initial_changes.len() < limit {
            return Ok(initial_changes);
        }

        // A source transaction may contain several changes with one version. Re-read through
        // the complete boundary version so the caller never acknowledges a partial transaction.
        let complete_query = format!(
            "SELECT migration_id, source_identity, source_version, table_name, stable_key,
                    previous_key, kind, payload, payload_digest
               FROM {capture_table}
              WHERE migration_id = ?
                AND source_version > ? AND source_version <= ?
              ORDER BY source_version ASC, capture_id ASC"
        );
        sqlx::query(&complete_query)
            .bind(&self.migration_id)
            .bind(after)
            .bind(boundary_version)
            .fetch_all(&self.pool)
            .await
            .context("read complete SQLite migration change boundary")?
            .into_iter()
            .map(|row| decode_sqlite_change(row, &self.migration_id, &self.source_identity))
            .collect()
    }
}

fn decode_sqlite_change(
    row: SqliteRow,
    migration_id: &str,
    source_identity: &str,
) -> Result<CapturedChange> {
    let change = CapturedChange {
        migration_id: row.try_get("migration_id")?,
        source_identity: row.try_get("source_identity")?,
        source_version: row.try_get("source_version")?,
        table_name: row.try_get("table_name")?,
        stable_key: row.try_get("stable_key")?,
        previous_key: row.try_get("previous_key")?,
        kind: parse_change_kind(row.try_get::<String, _>("kind")?.as_str())?,
        payload: row.try_get("payload")?,
        payload_digest: row.try_get("payload_digest")?,
    };
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
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query = format!(
            "SELECT COALESCE(MAX(source_version), 0)
               FROM {capture_table}
              WHERE migration_id = ? AND source_identity = ?"
        );
        Ok(sqlx::query_scalar(&query)
            .bind(&self.migration_id)
            .bind(&self.source_identity)
            .fetch_one(&self.pool)
            .await
            .context("read SQLite migration capture watermark")?)
    }

    async fn module_tables(&self, module: StorageModule) -> Result<Vec<String>> {
        let tables = module_table_names(&self.specs, module);
        if tables.is_empty() {
            bail!("SQLite migration source has no mapped tables for {module:?}");
        }
        Ok(tables)
    }

    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)> {
        let spec = self.spec(table_name)?;
        let table = quote_identifier(&spec.source_table_name, "source migration table")?;
        let key = quote_identifier(&spec.source_key_column, "source stable-key column")?;
        let query =
            format!("SELECT MIN({key}) AS lower_bound, MAX({key}) AS upper_bound FROM {table}");
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
        if limits.max_rows == 0 || limits.max_bytes == 0 {
            bail!("SQLite migration page limits must be positive");
        }
        let Some(upper_bound) = upper_bound else {
            return Ok(SourcePage {
                rows: Vec::new(),
                next_cursor: None,
                complete: true,
            });
        };
        let spec = self.spec(table_name)?;
        let table = quote_identifier(&spec.source_table_name, "source migration table")?;
        let key = quote_identifier(&spec.source_key_column, "source stable-key column")?;
        let version = quote_identifier(&spec.source_version_column, "source version column")?;
        let payload = quote_identifier(&spec.source_payload_column, "source payload column")?;
        let query = format!(
            "SELECT {key} AS stable_key, {version} AS source_version, {payload} AS payload
               FROM {table}
              WHERE {key} > ? AND {key} <= ?
              ORDER BY {key} ASC LIMIT ?"
        );
        let raw_rows = sqlx::query(&query)
            .bind(cursor.unwrap_or_default())
            .bind(upper_bound)
            .bind(i64::try_from(limits.max_rows).context("SQLite page row limit overflowed")?)
            .fetch_all(&self.pool)
            .await
            .context("read SQLite migration baseline page")?;
        let fetched_rows = raw_rows
            .into_iter()
            .map(|row| {
                Ok(VersionedRow {
                    stable_key: row.try_get("stable_key")?,
                    source_version: row.try_get("source_version")?,
                    payload: row
                        .try_get::<Option<Vec<u8>>, _>("payload")?
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let fetched_count = fetched_rows.len();
        let mut bytes = 0_usize;
        let mut rows = Vec::with_capacity(fetched_count);
        for row in fetched_rows {
            if rows.is_empty() && row.payload.len() > limits.max_bytes {
                bail!("SQLite migration row exceeds the copy byte budget");
            }
            let next_bytes = bytes
                .checked_add(row.payload.len())
                .ok_or_else(|| anyhow!("SQLite migration page byte count overflowed"))?;
            if next_bytes > limits.max_bytes {
                break;
            }
            bytes = next_bytes;
            rows.push(row);
        }
        let byte_limited = rows.len() < fetched_count;
        let next_cursor = rows.last().map(|row| row.stable_key.clone());
        Ok(SourcePage {
            complete: rows.is_empty()
                || (!byte_limited
                    && (fetched_count < limits.max_rows
                        || next_cursor.as_deref() == Some(upper_bound))),
            rows,
            next_cursor,
        })
    }

    async fn rows_in_range_snapshot(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<SourceRangeSnapshot> {
        let spec = self.spec(table_name)?;
        let table = quote_identifier(&spec.source_table_name, "source migration table")?;
        let key = quote_identifier(&spec.source_key_column, "source stable-key column")?;
        let version = quote_identifier(&spec.source_version_column, "source version column")?;
        let payload = quote_identifier(&spec.source_payload_column, "source payload column")?;
        let capture_table =
            quote_identifier(&self.capture_table_name, "SQLite migration capture table")?;
        let query = format!(
            "SELECT {key} AS stable_key, {version} AS source_version, {payload} AS payload
               FROM {table}
              WHERE {key} >= ? AND {key} <= ?
              ORDER BY {key} ASC"
        );
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(&query)
            .bind(&range.first_key)
            .bind(&range.last_key)
            .fetch_all(&mut *transaction)
            .await
            .context("read SQLite migration verification range")?;
        let rows = rows
            .into_iter()
            .map(|row| {
                Ok(VersionedRow {
                    stable_key: row.try_get("stable_key")?,
                    source_version: row.try_get("source_version")?,
                    payload: row
                        .try_get::<Option<Vec<u8>>, _>("payload")?
                        .unwrap_or_default(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
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
            .context("read SQLite verification snapshot watermark")?;
        transaction
            .commit()
            .await
            .context("commit SQLite migration verification snapshot")?;
        Ok(SourceRangeSnapshot {
            source_watermark,
            rows,
        })
    }

    async fn changes_after(&self, after: i64, limit: usize) -> Result<Vec<CapturedChange>> {
        self.read_changes(after, i64::MAX, limit).await
    }

    async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        self.read_changes(after, through, limit).await
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
}

#[derive(Debug, Clone)]
struct PostgresMigrationMetadataTables {
    plans: String,
    checkpoints: String,
    chunk_proofs: String,
    coverage_proofs: String,
}

impl PostgresMigrationMetadataTables {
    fn new(prefix: &str) -> Result<Self> {
        validate_identifier(prefix, "PostgreSQL migration metadata prefix")?;
        Ok(Self {
            plans: format!("{prefix}_plans"),
            checkpoints: format!("{prefix}_checkpoints"),
            chunk_proofs: format!("{prefix}_chunk_proofs"),
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
        })
    }

    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let chunk_proofs = quote_identifier(
            &self.metadata.chunk_proofs,
            "PostgreSQL migration chunk proofs table",
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
                "CREATE TABLE IF NOT EXISTS {coverage_proofs} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
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
        Ok(())
    }

    fn spec(&self, table_name: &str) -> Result<&MigrationTableSpec> {
        self.specs
            .get(table_name)
            .ok_or_else(|| anyhow!("PostgreSQL migration table is not mapped: {table_name}"))
    }

    async fn load_plan_tx(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCopyPlan>> {
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let query = format!(
            "SELECT migration_id, table_name, scan_lower_bound, scan_upper_bound, capture_start
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
            "SELECT migration_id, table_name, cursor, scan_upper_bound, capture_start,
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
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let query = format!(
            "INSERT INTO {checkpoints}
             (migration_id, table_name, cursor, scan_upper_bound, capture_start,
              captured_through, applied_through, copied_rows, copied_bytes, complete)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (migration_id, table_name) DO UPDATE SET
                cursor = EXCLUDED.cursor, scan_upper_bound = EXCLUDED.scan_upper_bound,
                capture_start = EXCLUDED.capture_start, captured_through = EXCLUDED.captured_through,
                applied_through = EXCLUDED.applied_through, copied_rows = EXCLUDED.copied_rows,
                copied_bytes = EXCLUDED.copied_bytes, complete = EXCLUDED.complete"
        );
        sqlx::query(&query)
            .bind(&checkpoint.migration_id)
            .bind(&checkpoint.table_name)
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
            "SELECT migration_id, table_name, scan_lower_bound, scan_upper_bound, capture_start
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
        if plan.capture_start < 0 {
            bail!("PostgreSQL migration plan capture start must not be negative");
        }
        let plans = quote_identifier(&self.metadata.plans, "PostgreSQL migration plans table")?;
        let checkpoints = quote_identifier(
            &self.metadata.checkpoints,
            "PostgreSQL migration checkpoints table",
        )?;
        let mut transaction = self.pool.begin().await?;
        let query = format!(
            "INSERT INTO {plans}
             (migration_id, table_name, scan_lower_bound, scan_upper_bound, capture_start)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (migration_id, table_name) DO NOTHING"
        );
        sqlx::query(&query)
            .bind(&plan.migration_id)
            .bind(&plan.table_name)
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
             (migration_id, table_name, cursor, scan_upper_bound, capture_start,
              captured_through, applied_through, copied_rows, copied_bytes, complete)
             VALUES ($1, $2, NULL, $3, $4, $4, $4, 0, 0, FALSE)
             ON CONFLICT (migration_id, table_name) DO NOTHING"
        );
        sqlx::query(&checkpoint_query)
            .bind(&plan.migration_id)
            .bind(&plan.table_name)
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
            "SELECT migration_id, table_name, cursor, scan_upper_bound, capture_start,
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
        if checkpoint.migration_id != plan.migration_id || checkpoint.table_name != plan.table_name
        {
            bail!("PostgreSQL baseline checkpoint does not match its plan");
        }
        let mut transaction = self.pool.begin().await?;
        let existing = self
            .load_checkpoint_tx(&mut transaction, &plan.migration_id, &plan.table_name)
            .await?
            .ok_or_else(|| anyhow!("PostgreSQL migration checkpoint is missing"))?;
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
        if checkpoint.migration_id != migration_id {
            bail!("PostgreSQL replay checkpoint does not match its migration");
        }
        let mut transaction = self.pool.begin().await?;
        let existing = self
            .load_checkpoint_tx(&mut transaction, migration_id, &checkpoint.table_name)
            .await?
            .ok_or_else(|| anyhow!("PostgreSQL migration checkpoint is missing"))?;
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

    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>> {
        let spec = self.spec(table_name)?;
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let version = quote_identifier(&spec.target_version_column, "target version column")?;
        let payload = quote_identifier(&spec.target_payload_column, "target payload column")?;
        let query = format!(
            "SELECT {key} AS stable_key, {version} AS source_version, {payload} AS payload
               FROM {table}
              WHERE {key} >= $1 AND {key} <= $2
              ORDER BY {key} ASC"
        );
        let rows = sqlx::query(&query)
            .bind(&range.first_key)
            .bind(&range.last_key)
            .fetch_all(&self.pool)
            .await
            .context("read PostgreSQL migration verification range")?;
        rows.into_iter()
            .map(|row| {
                Ok(VersionedRow {
                    stable_key: row.try_get("stable_key")?,
                    source_version: row.try_get("source_version")?,
                    payload: row
                        .try_get::<Option<Vec<u8>>, _>("payload")?
                        .unwrap_or_default(),
                })
            })
            .collect()
    }

    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)> {
        let spec = self.spec(table_name)?;
        let table = quote_identifier(&spec.target_table_name, "target migration table")?;
        let key = quote_identifier(&spec.target_key_column, "target stable-key column")?;
        let query =
            format!("SELECT MIN({key}) AS lower_bound, MAX({key}) AS upper_bound FROM {table}");
        let row = sqlx::query(&query)
            .fetch_one(&self.pool)
            .await
            .context("read PostgreSQL target table bounds")?;
        Ok((row.try_get("lower_bound")?, row.try_get("upper_bound")?))
    }

    async fn save_chunk_proof(&self, proof: &ChunkProof) -> Result<()> {
        if proof.migration_id.trim().is_empty()
            || proof.table_name.trim().is_empty()
            || proof.source_watermark < 0
            || proof.range.first_key > proof.range.last_key
        {
            bail!("invalid PostgreSQL migration chunk proof");
        }
        let chunk_proofs = quote_identifier(
            &self.metadata.chunk_proofs,
            "PostgreSQL migration chunk proofs table",
        )?;
        let query = format!(
            "INSERT INTO {chunk_proofs}
             (migration_id, table_name, first_key, last_key, source_watermark, row_count, byte_count, digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (migration_id, table_name, first_key, last_key) DO UPDATE SET
                source_watermark = EXCLUDED.source_watermark,
                row_count = EXCLUDED.row_count, byte_count = EXCLUDED.byte_count,
                digest = EXCLUDED.digest"
        );
        sqlx::query(&query)
            .bind(&proof.migration_id)
            .bind(&proof.table_name)
            .bind(&proof.range.first_key)
            .bind(&proof.range.last_key)
            .bind(proof.source_watermark)
            .bind(i64_from_u64(proof.row_count, "chunk proof row count")?)
            .bind(i64_from_u64(proof.byte_count, "chunk proof byte count")?)
            .bind(&proof.digest)
            .execute(&self.pool)
            .await
            .context("persist PostgreSQL migration chunk proof")?;
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
            "SELECT migration_id, table_name, first_key, last_key, source_watermark,
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
                Ok(ChunkProof {
                    migration_id: row.try_get("migration_id")?,
                    table_name: row.try_get("table_name")?,
                    range: ChunkRange::new(
                        row.try_get::<String, _>("first_key")?,
                        row.try_get::<String, _>("last_key")?,
                    )?,
                    source_watermark: row.try_get("source_watermark")?,
                    row_count: u64_from_i64(row.try_get("row_count")?, "chunk proof row count")?,
                    byte_count: u64_from_i64(row.try_get("byte_count")?, "chunk proof byte count")?,
                    digest: row.try_get("digest")?,
                })
            })
            .collect()
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
             (migration_id, table_name, source_watermark, range_first_key, range_last_key,
              source_lower_bound, source_upper_bound, target_lower_bound, target_upper_bound,
              source_row_count, target_row_count, source_byte_count, target_byte_count,
              source_digest, target_digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (migration_id, table_name) DO UPDATE SET
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
                target_digest = EXCLUDED.target_digest"
        );
        let mut transaction = self.pool.begin().await?;
        sqlx::query(&query)
            .bind(&proof.migration_id)
            .bind(&proof.table_name)
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
            "SELECT migration_id, table_name, source_watermark, range_first_key, range_last_key,
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
    Ok(TableCopyPlan {
        migration_id: row.try_get("migration_id")?,
        table_name: row.try_get("table_name")?,
        scan_lower_bound: row.try_get("scan_lower_bound")?,
        scan_upper_bound: row.try_get("scan_upper_bound")?,
        capture_start: row.try_get("capture_start")?,
    })
}

fn decode_checkpoint(row: sqlx::postgres::PgRow) -> Result<EngineCheckpoint> {
    Ok(EngineCheckpoint {
        migration_id: row.try_get("migration_id")?,
        table_name: row.try_get("table_name")?,
        cursor: row.try_get("cursor")?,
        scan_upper_bound: row.try_get("scan_upper_bound")?,
        capture_start: row.try_get("capture_start")?,
        captured_through: row.try_get("captured_through")?,
        applied_through: row.try_get("applied_through")?,
        copied_rows: u64_from_i64(row.try_get("copied_rows")?, "copied rows")?,
        copied_bytes: u64_from_i64(row.try_get("copied_bytes")?, "copied bytes")?,
        complete: row.try_get("complete")?,
    })
}

fn i64_from_u64(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value).with_context(|| format!("{label} exceeds PostgreSQL BIGINT"))
}

fn u64_from_i64(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value).with_context(|| format!("{label} is negative"))
}
