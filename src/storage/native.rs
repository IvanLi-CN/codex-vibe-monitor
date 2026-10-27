//! Native PostgreSQL schema and row ports for module-owned migration.
//!
//! The generic migration engine still has a compact payload fixture for protocol tests. This
//! module is the production-facing shape for the approved module catalog: every source column is
//! represented explicitly, and the target DDL keeps primary keys, unique constraints, checks,
//! foreign keys, resource paths, and the capture-sequence watermark separate from row data.

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row, postgres::PgRow, sqlite::SqliteRow};

use super::{
    analytics::ensure_analytics_projection_checkpoint_schema,
    archive::{ARCHIVE_STORAGE_TABLES, ensure_archive_storage_schema},
    catalog::{CatalogError, NativeColumnType, NativeMigrationCatalog, NativeTableSpec},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum NativeValue {
    Null,
    BigInt(i64),
    Double(f64),
    Boolean(bool),
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NativeRow {
    table_name: String,
    values: Vec<(String, NativeValue)>,
}

impl NativeRow {
    pub(crate) fn from_values(
        spec: &NativeTableSpec,
        values: impl IntoIterator<Item = NativeValue>,
    ) -> Result<Self> {
        let values = values.into_iter().collect::<Vec<_>>();
        if values.len() != spec.columns.len() {
            bail!(
                "native row for {} has {} values, expected {}",
                spec.table_name(),
                values.len(),
                spec.columns.len()
            );
        }
        let row = Self {
            table_name: spec.table_name().to_owned(),
            values: spec
                .columns
                .iter()
                .map(|column| column.name.clone())
                .zip(values)
                .collect(),
        };
        row.validate(spec)?;
        Ok(row)
    }

    pub(crate) fn from_sqlite_row(spec: &NativeTableSpec, row: &SqliteRow) -> Result<Self> {
        let values = spec
            .columns
            .iter()
            .map(|column| {
                let value = match column.postgres_type {
                    NativeColumnType::BigInt => row
                        .try_get::<Option<i64>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::BigInt))?,
                    NativeColumnType::DoublePrecision => row
                        .try_get::<Option<f64>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Double))?,
                    NativeColumnType::Boolean => row
                        .try_get::<Option<i64>, _>(column.name.as_str())
                        .map(|value| value.map(|value| NativeValue::Boolean(value != 0)))?,
                    NativeColumnType::Text => row
                        .try_get::<Option<String>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Text))?,
                    NativeColumnType::Bytea => row
                        .try_get::<Option<Vec<u8>>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Bytes))?,
                };
                Ok(value.unwrap_or(NativeValue::Null))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Self::from_values(spec, values)
    }

    pub(crate) fn from_postgres_row(spec: &NativeTableSpec, row: &PgRow) -> Result<Self> {
        let values = spec
            .columns
            .iter()
            .map(|column| {
                let value = match column.postgres_type {
                    NativeColumnType::BigInt => row
                        .try_get::<Option<i64>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::BigInt))?,
                    NativeColumnType::DoublePrecision => row
                        .try_get::<Option<f64>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Double))?,
                    NativeColumnType::Boolean => row
                        .try_get::<Option<bool>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Boolean))?,
                    NativeColumnType::Text => row
                        .try_get::<Option<String>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Text))?,
                    NativeColumnType::Bytea => row
                        .try_get::<Option<Vec<u8>>, _>(column.name.as_str())
                        .map(|value| value.map(NativeValue::Bytes))?,
                };
                Ok(value.unwrap_or(NativeValue::Null))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Self::from_values(spec, values)
    }

    pub(crate) fn encode_payload(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).context("encode native migration row")
    }

    pub(crate) fn decode_payload(spec: &NativeTableSpec, payload: &[u8]) -> Result<Self> {
        if let Ok(row) = serde_json::from_slice::<Self>(payload) {
            row.validate(spec)?;
            return Ok(row);
        }

        // Capture triggers use SQLite's JSON functions so they can observe every writer,
        // including legacy SQL paths. Decode that map into the same typed native row contract.
        let object = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(payload)
            .context("decode native migration row")?;
        let values = spec
            .columns
            .iter()
            .map(|column| {
                let value = object.get(&column.name).unwrap_or(&serde_json::Value::Null);
                let value = match (column.postgres_type, value) {
                    (_, serde_json::Value::Null) => NativeValue::Null,
                    (NativeColumnType::BigInt, value) => {
                        NativeValue::BigInt(value.as_i64().ok_or_else(|| {
                            anyhow!("native capture value for {} is not BIGINT", column.name)
                        })?)
                    }
                    (NativeColumnType::DoublePrecision, value) => {
                        NativeValue::Double(value.as_f64().ok_or_else(|| {
                            anyhow!("native capture value for {} is not DOUBLE", column.name)
                        })?)
                    }
                    (NativeColumnType::Boolean, value) => NativeValue::Boolean(
                        value
                            .as_bool()
                            .or_else(|| value.as_i64().map(|value| value != 0))
                            .ok_or_else(|| {
                                anyhow!("native capture value for {} is not BOOLEAN", column.name)
                            })?,
                    ),
                    (NativeColumnType::Text, value) => NativeValue::Text(
                        value
                            .as_str()
                            .ok_or_else(|| {
                                anyhow!("native capture value for {} is not TEXT", column.name)
                            })?
                            .to_owned(),
                    ),
                    (NativeColumnType::Bytea, value) => {
                        let encoded = value
                            .get("__cvm_blob_hex")
                            .and_then(serde_json::Value::as_str)
                            .ok_or_else(|| {
                                anyhow!("native capture value for {} is not BYTEA", column.name)
                            })?;
                        NativeValue::Bytes(decode_hex(encoded)?)
                    }
                };
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        Self::from_values(spec, values)
    }

    pub(crate) fn table_name(&self) -> &str {
        &self.table_name
    }

    pub(crate) fn values(&self) -> &[(String, NativeValue)] {
        &self.values
    }

    fn validate(&self, spec: &NativeTableSpec) -> Result<()> {
        if self.table_name != spec.table_name() {
            bail!(
                "native row belongs to {}, expected {}",
                self.table_name,
                spec.table_name()
            );
        }
        let values = self
            .values
            .iter()
            .map(|(column, value)| (column.as_str(), value))
            .collect::<std::collections::BTreeMap<_, _>>();
        for column in &spec.columns {
            let value = values
                .get(column.name.as_str())
                .ok_or_else(|| anyhow!("native row is missing column {}", column.name))?;
            if !column.nullable && matches!(value, NativeValue::Null) {
                bail!("native row has NULL in non-null column {}", column.name);
            }
            if !matches!(value, NativeValue::Null)
                && !native_value_matches(column.postgres_type, value)
            {
                bail!(
                    "native row has the wrong value type in column {}",
                    column.name
                );
            }
        }
        for key in spec.stable_key_columns() {
            if matches!(values.get(key.as_str()), Some(NativeValue::Null) | None) {
                bail!("native row has an incomplete stable key at {key}");
            }
        }
        Ok(())
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let (chunks, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        bail!("native capture bytea hex has an odd length");
    }
    chunks
        .iter()
        .map(|chunk| {
            let high = (chunk[0] as char)
                .to_digit(16)
                .ok_or_else(|| anyhow!("native capture bytea hex is invalid"))?;
            let low = (chunk[1] as char)
                .to_digit(16)
                .ok_or_else(|| anyhow!("native capture bytea hex is invalid"))?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

fn native_value_matches(column_type: NativeColumnType, value: &NativeValue) -> bool {
    matches!(
        (column_type, value),
        (NativeColumnType::BigInt, NativeValue::BigInt(_))
            | (NativeColumnType::DoublePrecision, NativeValue::Double(_))
            | (NativeColumnType::Boolean, NativeValue::Boolean(_))
            | (NativeColumnType::Text, NativeValue::Text(_))
            | (NativeColumnType::Bytea, NativeValue::Bytes(_))
    )
}

#[derive(Debug, Clone)]
pub(crate) struct PostgresNativeSchemaAdapter {
    pool: PgPool,
    catalog: NativeMigrationCatalog,
}

impl PostgresNativeSchemaAdapter {
    pub(crate) fn new(pool: PgPool, catalog: NativeMigrationCatalog) -> Self {
        Self { pool, catalog }
    }

    pub(crate) fn catalog(&self) -> &NativeMigrationCatalog {
        &self.catalog
    }

    pub(crate) fn ddl_statements(&self) -> Result<Vec<String>, CatalogError> {
        let mut statements = Vec::new();
        for spec in &self.catalog.tables {
            if is_existing_native_table(spec.table_name()) {
                continue;
            }
            statements.push(spec.postgres_create_table_sql()?);
        }
        for spec in &self.catalog.tables {
            let existing_native_table = is_existing_native_table(spec.table_name());
            // Existing owner-specific tables use an explicit target contract. Their foreign keys
            // are part of that owner schema; replaying SQLite FK names here would, for example,
            // incorrectly reference archive_batches.id instead of archive_batch_id.
            if !spec
                .foreign_keys
                .iter()
                .any(|foreign_key| foreign_key.referenced_table == "archive_batches")
            {
                statements.extend(spec.postgres_foreign_key_sql()?);
            }
            if !existing_native_table {
                statements.extend(spec.postgres_index_sql()?);
            }
        }
        Ok(statements)
    }

    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        if self
            .catalog
            .modules
            .contains(&super::catalog::ModuleId::TrafficArchive)
        {
            ensure_archive_storage_schema(&self.pool).await?;
        }
        if self
            .catalog
            .modules
            .contains(&super::catalog::ModuleId::Analytics)
        {
            ensure_analytics_projection_checkpoint_schema(&self.pool).await?;
        }

        let statements = self.ddl_statements().map_err(|error| anyhow!(error))?;
        let mut transaction = self.pool.begin().await?;
        for statement in statements {
            sqlx::query(&statement)
                .execute(&mut *transaction)
                .await
                .context("create native PostgreSQL migration schema")?;
        }
        transaction
            .commit()
            .await
            .context("commit native PostgreSQL migration schema")?;
        Ok(())
    }
}

fn is_existing_native_table(table: &str) -> bool {
    table == "analytics_projection_checkpoints" || ARCHIVE_STORAGE_TABLES.contains(&table)
}

pub(crate) fn native_row_query_columns(spec: &NativeTableSpec) -> String {
    spec.columns
        .iter()
        .map(|column| format!("\"{}\"", column.name.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ")
}
