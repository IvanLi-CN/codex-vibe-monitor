use super::*;
use crate::migration::{postgres_type_for_source_column_for_store, quote_literal_for_store};
use crate::primary_db::PrimaryDatabase;

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

async fn business_key_predicate(
    pool: &sqlx::PgPool,
    run_id: &str,
    object: &str,
    key: &str,
    parameter: &str,
) -> Result<String> {
    let key: serde_json::Value =
        serde_json::from_str(key).context("source key must be a JSON business key")?;
    let columns: Vec<(String,String)> = sqlx::query_as("SELECT column_name,declared_type FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name=$2 AND primary_key_position>0 ORDER BY primary_key_position").bind(run_id).bind(object).fetch_all(pool).await?;
    if columns.is_empty() {
        bail!("business key is unavailable for {object}");
    }
    columns
        .into_iter()
        .map(|(column, declared_type)| {
            let value = key
                .get(&column)
                .with_context(|| format!("missing business key column {column}"))?;
            if value.is_null() {
                return Ok(format!("{} IS NULL", quote_identifier(&column)));
            }
            let pg_type = postgres_type_for_source_column_for_store(&declared_type);
            let field = format!(
                "{parameter}::jsonb ->> {}",
                quote_literal_for_store(&column)
            );
            let expression = if pg_type == "BYTEA" {
                format!("decode({field}, 'base64')")
            } else {
                format!("({field})::{pg_type}")
            };
            Ok(format!("{} = {expression}", quote_identifier(&column)))
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join(" AND "))
}

#[async_trait::async_trait]
pub(crate) trait PrimaryStateStore: Send + Sync + std::fmt::Debug {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>>;
    async fn put_json(&self, key: &str, value: &serde_json::Value) -> Result<()>;

    async fn put_json_batch(&self, values: &[(&str, serde_json::Value)]) -> Result<()> {
        for (key, value) in values {
            self.put_json(key, value).await?;
        }
        Ok(())
    }

    async fn append_json_array(
        &self,
        key: &str,
        value: serde_json::Value,
        max_len: usize,
    ) -> Result<()> {
        let mut values = self
            .get_json(key)
            .await?
            .and_then(|existing| existing.as_array().cloned())
            .unwrap_or_default();
        values.push(value);
        if values.len() > max_len {
            values.drain(0..values.len() - max_len);
        }
        self.put_json(key, &serde_json::Value::Array(values)).await
    }

    async fn latest_materialized_rows(&self, _object_name: &str) -> Result<Vec<serde_json::Value>> {
        bail!("materialized domain rows are unavailable for this primary backend")
    }

    async fn delete_materialized_rows_by_source_keys(
        &self,
        _object_name: &str,
        _source_keys: &[String],
    ) -> Result<u64> {
        bail!("materialized domain deletes are unavailable for this primary backend")
    }

    async fn update_materialized_text_fields(
        &self,
        _object_name: &str,
        _source_key: &str,
        _fields: &[(&str, &str)],
    ) -> Result<()> {
        bail!("materialized domain updates are unavailable for this primary backend")
    }

    async fn update_materialized_integer_fields(
        &self,
        _object_name: &str,
        _source_key: &str,
        _fields: &[(&str, i64)],
    ) -> Result<()> {
        bail!("materialized integer updates are unavailable for this primary backend")
    }

    async fn update_materialized_float_fields(
        &self,
        _object_name: &str,
        _source_key: &str,
        _fields: &[(&str, f64)],
    ) -> Result<()> {
        bail!("materialized float updates are unavailable for this primary backend")
    }

    async fn mutate_materialized_account(
        &self,
        _account_id: i64,
        _action: &str,
        _group_name: Option<&str>,
    ) -> Result<Option<serde_json::Value>> {
        bail!("materialized account writes are unavailable for this primary backend")
    }

    async fn update_materialized_account(
        &self,
        _account_id: i64,
        _patch: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        bail!("materialized account writes are unavailable for this primary backend")
    }

    async fn create_materialized_account(
        &self,
        _payload: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        bail!("materialized account writes are unavailable for this primary backend")
    }

    async fn record_proxy_request(&self, _record: ProxyRequestRecord) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyRequestRecord {
    pub(crate) invoke_id: String,
    pub(crate) method: String,
    pub(crate) request_path: String,
    pub(crate) status_code: Option<u16>,
    pub(crate) request_bytes: usize,
    pub(crate) response_bytes: usize,
    pub(crate) duration_ms: f64,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SqlitePrimaryStateStore {
    pool: Pool<Sqlite>,
}

impl SqlitePrimaryStateStore {
    pub(crate) fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }

    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS cvm_primary_state (\
                state_key TEXT PRIMARY KEY,\
                state_value TEXT NOT NULL,\
                updated_at TEXT NOT NULL\
            )",
        )
        .execute(&self.pool)
        .await
        .context("failed to ensure SQLite primary state store schema")?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl PrimaryStateStore for SqlitePrimaryStateStore {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT state_value FROM cvm_primary_state WHERE state_key=?1")
                .bind(key)
                .fetch_optional(&self.pool)
                .await
                .context("failed to read SQLite primary JSON state")?;
        value
            .map(|value| {
                serde_json::from_str(&value)
                    .with_context(|| format!("invalid JSON in SQLite primary state key {key}"))
            })
            .transpose()
    }

    async fn put_json(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let encoded = serde_json::to_string(value)?;
        sqlx::query(
            "INSERT INTO cvm_primary_state(state_key,state_value,updated_at) VALUES(?1,?2,datetime('now')) \
             ON CONFLICT(state_key) DO UPDATE SET state_value=excluded.state_value,updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(encoded)
        .execute(&self.pool)
        .await
        .context("failed to write SQLite primary JSON state")?;
        Ok(())
    }

    async fn put_json_batch(&self, values: &[(&str, serde_json::Value)]) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("failed to begin SQLite primary state batch")?;
        for (key, value) in values {
            let encoded = serde_json::to_string(value)?;
            sqlx::query(
                "INSERT INTO cvm_primary_state(state_key,state_value,updated_at) VALUES(?1,?2,datetime('now')) \
                 ON CONFLICT(state_key) DO UPDATE SET state_value=excluded.state_value,updated_at=excluded.updated_at",
            )
            .bind(*key)
            .bind(encoded)
            .execute(&mut *tx)
            .await
            .context("failed to write SQLite primary JSON state batch")?;
        }
        tx.commit()
            .await
            .context("failed to commit SQLite primary state batch")?;
        Ok(())
    }

    async fn append_json_array(
        &self,
        key: &str,
        value: serde_json::Value,
        max_len: usize,
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("failed to begin SQLite primary JSON array append")?;
        let existing: Option<String> =
            sqlx::query_scalar("SELECT state_value FROM cvm_primary_state WHERE state_key=?1")
                .bind(key)
                .fetch_optional(&mut *tx)
                .await
                .context("failed to lock SQLite primary JSON array")?;
        let mut values = existing
            .and_then(|encoded| serde_json::from_str::<serde_json::Value>(&encoded).ok())
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        values.push(value);
        if values.len() > max_len {
            values.drain(0..values.len() - max_len);
        }
        let encoded = serde_json::to_string(&serde_json::Value::Array(values))?;
        sqlx::query(
            "INSERT INTO cvm_primary_state(state_key,state_value,updated_at) VALUES(?1,?2,datetime('now')) ON CONFLICT(state_key) DO UPDATE SET state_value=excluded.state_value,updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(encoded)
        .execute(&mut *tx)
        .await
        .context("failed to write SQLite primary JSON array")?;
        tx.commit()
            .await
            .context("failed to commit SQLite primary JSON array append")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PostgresPrimaryStateStore {
    database: PrimaryDatabase,
}

impl PostgresPrimaryStateStore {
    pub(crate) fn new(database: PrimaryDatabase) -> Self {
        Self { database }
    }
}

#[async_trait::async_trait]
impl PrimaryStateStore for PostgresPrimaryStateStore {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>> {
        self.database.get_json_state(key).await
    }

    async fn put_json(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        self.database.put_json_state(key, value).await
    }

    async fn put_json_batch(&self, values: &[(&str, serde_json::Value)]) -> Result<()> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .context("failed to begin PostgreSQL primary state batch")?;
        for (key, value) in values {
            sqlx::query(
                "INSERT INTO cvm_core.state_kv(state_key,state_value) VALUES($1,$2) \
                 ON CONFLICT(state_key) DO UPDATE SET state_value=EXCLUDED.state_value,updated_at=now()",
            )
            .bind(*key)
            .bind(sqlx::types::Json(value))
            .execute(&mut *tx)
            .await
            .context("failed to write PostgreSQL primary JSON state batch")?;
        }
        tx.commit()
            .await
            .context("failed to commit PostgreSQL primary state batch")?;
        Ok(())
    }

    async fn append_json_array(
        &self,
        key: &str,
        value: serde_json::Value,
        max_len: usize,
    ) -> Result<()> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .context("failed to begin PostgreSQL primary JSON array append")?;
        let existing: Option<sqlx::types::Json<serde_json::Value>> = sqlx::query_scalar(
            "SELECT state_value FROM cvm_core.state_kv WHERE state_key=$1 FOR UPDATE",
        )
        .bind(key)
        .fetch_optional(&mut *tx)
        .await
        .context("failed to lock PostgreSQL primary JSON array")?;
        let mut values = existing
            .and_then(|encoded| encoded.0.as_array().cloned())
            .unwrap_or_default();
        values.push(value);
        if values.len() > max_len {
            values.drain(0..values.len() - max_len);
        }
        sqlx::query(
            "INSERT INTO cvm_core.state_kv(state_key,state_value) VALUES($1,$2) ON CONFLICT(state_key) DO UPDATE SET state_value=EXCLUDED.state_value,updated_at=now()",
        )
        .bind(key)
        .bind(sqlx::types::Json(serde_json::Value::Array(values)))
        .execute(&mut *tx)
        .await
        .context("failed to write PostgreSQL primary JSON array")?;
        tx.commit()
            .await
            .context("failed to commit PostgreSQL primary JSON array append")
    }

    async fn latest_materialized_rows(&self, object_name: &str) -> Result<Vec<serde_json::Value>> {
        if object_name == "proxy_requests" {
            let rows: Vec<sqlx::types::Json<serde_json::Value>> = sqlx::query_scalar(
                "SELECT to_jsonb(proxy_request) FROM cvm_activity.proxy_requests AS proxy_request ORDER BY occurred_at DESC, id DESC",
            )
            .fetch_all(self.database.pool())
            .await
            .context("failed to read PostgreSQL primary proxy requests")?;
            return Ok(rows.into_iter().map(|row| row.0).collect());
        }
        let run_id: Option<String> = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(self.database.pool())
        .await
        .context("failed to select the latest verified migration run")?;
        let Some(run_id) = run_id else {
            return Ok(Vec::new());
        };
        if object_name == "asset_index" {
            let rows: Vec<sqlx::types::Json<serde_json::Value>> = sqlx::query_scalar(
                "SELECT to_jsonb(asset) FROM cvm_assets.asset_index AS asset WHERE asset.run_id=$1 ORDER BY asset.relative_path",
            )
            .bind(&run_id)
            .fetch_all(self.database.pool())
            .await
            .context("failed to read PostgreSQL primary asset index")?;
            return Ok(rows.into_iter().map(|row| row.0).collect());
        }
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(&run_id)
        .bind(object_name)
        .fetch_optional(self.database.pool())
        .await?
        .unwrap_or_else(|| ("cvm_core".to_owned(), object_name.to_owned()));
        let qualified = format!(
            "{}{}{}",
            quote_identifier(&schema),
            ".",
            quote_identifier(&table)
        );
        let rows: Vec<sqlx::types::Json<serde_json::Value>> = sqlx::query_scalar(&format!(
            "SELECT to_jsonb(materialized_row) FROM {qualified} AS materialized_row"
        ))
        .fetch_all(self.database.pool())
        .await
        .with_context(|| format!("failed to read materialized PostgreSQL object {object_name}"))?;
        Ok(rows.into_iter().map(|row| row.0).collect())
    }

    async fn delete_materialized_rows_by_source_keys(
        &self,
        object_name: &str,
        source_keys: &[String],
    ) -> Result<u64> {
        if source_keys.is_empty() {
            return Ok(0);
        }
        let pool = self.database.pool();
        let run_id: String = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("failed to select the latest verified migration run for materialized delete")?
        .context("no verified migration run is available for materialized delete")?;
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(&run_id)
        .bind(object_name)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("failed to resolve materialized target {object_name}"))?
        .with_context(|| format!("materialized target {object_name} is unavailable"))?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let keys = source_keys
            .iter()
            .map(|key| serde_json::from_str::<serde_json::Value>(key))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let columns: Vec<(String,String)> = sqlx::query_as("SELECT column_name,declared_type FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name=$2 AND primary_key_position>0 ORDER BY primary_key_position").bind(&run_id).bind(object_name).fetch_all(pool).await?;
        if columns.is_empty() {
            bail!("business key is unavailable for {object_name}");
        }
        let mut definitions = Vec::new();
        let mut predicates = Vec::new();
        for (column, declared_type) in columns {
            for key in &keys {
                if key.get(&column).is_none() {
                    bail!("missing business key column {column}");
                }
            }
            let pg_type = postgres_type_for_source_column_for_store(&declared_type);
            let name = quote_identifier(&column);
            definitions.push(format!(
                "{name} {}",
                if pg_type == "BYTEA" { "TEXT" } else { pg_type }
            ));
            let value = if pg_type == "BYTEA" {
                format!("decode(keys.{name},'base64')")
            } else {
                format!("keys.{name}")
            };
            let operator = if keys.iter().any(|key| key[&column].is_null()) {
                "IS NOT DISTINCT FROM"
            } else {
                "="
            };
            predicates.push(format!("actual.{name} {operator} {value}"));
        }
        let result=sqlx::query(&format!("DELETE FROM {qualified} actual USING jsonb_to_recordset($1::jsonb) AS keys({}) WHERE {}",definitions.join(", "),predicates.join(" AND "))).bind(sqlx::types::Json(keys)).execute(pool).await?;
        Ok(result.rows_affected())
    }

    async fn update_materialized_text_fields(
        &self,
        object_name: &str,
        source_key: &str,
        fields: &[(&str, &str)],
    ) -> Result<()> {
        if fields.is_empty() {
            return Ok(());
        }
        let pool = self.database.pool();
        let run_id: String = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("failed to select the latest verified migration run for materialized update")?
        .context("no verified migration run is available for materialized update")?;
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(&run_id)
        .bind(object_name)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("failed to resolve materialized target {object_name}"))?
        .with_context(|| format!("materialized target {object_name} is unavailable"))?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let predicate =
            business_key_predicate(pool, &run_id, object_name, source_key, "$2").await?;
        let assignments = fields
            .iter()
            .enumerate()
            .map(|(index, (field, _))| format!("{} = ${}", quote_identifier(field), index + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let statement = format!(
            "UPDATE {qualified} SET {assignments} WHERE $1::text IS NOT NULL AND {predicate}"
        );
        let mut query = sqlx::query(&statement).bind(&run_id).bind(source_key);
        for (_, value) in fields {
            query = query.bind(*value);
        }
        query
            .execute(pool)
            .await
            .with_context(|| format!("failed to update materialized fields in {object_name}"))?;
        Ok(())
    }

    async fn update_materialized_integer_fields(
        &self,
        object_name: &str,
        source_key: &str,
        fields: &[(&str, i64)],
    ) -> Result<()> {
        if fields.is_empty() {
            return Ok(());
        }
        let pool = self.database.pool();
        let run_id: String = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
        .context("no verified migration run is available for materialized integer update")?;
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(&run_id)
        .bind(object_name)
        .fetch_one(pool)
        .await?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let predicate =
            business_key_predicate(pool, &run_id, object_name, source_key, "$2").await?;
        let assignments = fields
            .iter()
            .enumerate()
            .map(|(index, (field, _))| format!("{} = ${}", quote_identifier(field), index + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let statement = format!(
            "UPDATE {qualified} SET {assignments} WHERE $1::text IS NOT NULL AND {predicate}"
        );
        let mut query = sqlx::query(&statement).bind(&run_id).bind(source_key);
        for (_, value) in fields {
            query = query.bind(*value);
        }
        query.execute(pool).await?;
        Ok(())
    }

    async fn update_materialized_float_fields(
        &self,
        object_name: &str,
        source_key: &str,
        fields: &[(&str, f64)],
    ) -> Result<()> {
        if fields.is_empty() {
            return Ok(());
        }
        let pool = self.database.pool();
        let run_id: String = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await?
        .context("no verified migration run is available for materialized float update")?;
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name=$2",
        )
        .bind(&run_id)
        .bind(object_name)
        .fetch_one(pool)
        .await?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let predicate =
            business_key_predicate(pool, &run_id, object_name, source_key, "$2").await?;
        let assignments = fields
            .iter()
            .enumerate()
            .map(|(index, (field, _))| format!("{} = ${}", quote_identifier(field), index + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let statement = format!(
            "UPDATE {qualified} SET {assignments} WHERE $1::text IS NOT NULL AND {predicate}"
        );
        let mut query = sqlx::query(&statement).bind(&run_id).bind(source_key);
        for (_, value) in fields {
            query = query.bind(*value);
        }
        query.execute(pool).await?;
        Ok(())
    }

    async fn mutate_materialized_account(
        &self,
        account_id: i64,
        action: &str,
        group_name: Option<&str>,
    ) -> Result<Option<serde_json::Value>> {
        let pool = self.database.pool();
        let run_id: Option<String> = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("failed to select the latest verified migration run for account write")?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name='pool_upstream_accounts'",
        )
        .bind(&run_id)
        .fetch_optional(pool)
        .await?
        .context("migrated upstream account object is unavailable")?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let account_kind: Option<String> = sqlx::query_scalar(&format!(
            "SELECT account.\"kind\"::text FROM {qualified} AS account WHERE $1::text IS NOT NULL AND account.\"id\"=$2"
        ))
        .bind(&run_id)
        .bind(account_id)
        .fetch_optional(pool)
        .await
        .context("failed to inspect materialized account kind")?;
        let Some(account_kind) = account_kind else {
            return Ok(None);
        };
        if action == "set_group" && account_kind == "api_key_codex" {
            bail!("API key accounts cannot be assigned to an upstream account group");
        }
        let (set_clause, bind_group): (&str, Option<&str>) = match action {
            "enable" => ("\"enabled\" = 1", None),
            "disable" => ("\"enabled\" = 0", None),
            "set_group" => ("\"group_name\" = NULLIF($3, '')", group_name),
            "delete" => ("\"deleted_at\" = now()::text", None),
            _ => bail!("unsupported materialized account action: {action}"),
        };
        let query = format!(
            "UPDATE {qualified} AS account SET {set_clause} WHERE $1::text IS NOT NULL AND account.\"id\"=$2 RETURNING to_jsonb(account)"
        );
        let mut request = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(&query)
            .bind(&run_id)
            .bind(account_id);
        if let Some(group_name) = bind_group {
            request = request.bind(group_name);
        }
        let row = request
            .fetch_optional(pool)
            .await
            .with_context(|| format!("failed to apply materialized account action {action}"))?;
        Ok(row.map(|value| value.0))
    }

    async fn update_materialized_account(
        &self,
        account_id: i64,
        patch: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        let patch = patch
            .as_object()
            .context("upstream account patch must be an object")?;
        let pool = self.database.pool();
        let run_id: Option<String> = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("failed to select the latest verified migration run for account patch")?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name='pool_upstream_accounts'",
        )
        .bind(&run_id)
        .fetch_optional(pool)
        .await?
        .context("migrated upstream account object is unavailable")?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT column_name, declared_type FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name='pool_upstream_accounts' ORDER BY ordinal",
        )
        .bind(&run_id)
        .fetch_all(pool)
        .await?;
        let mut assignments = Vec::new();
        let mut values = Vec::new();
        for (key, value) in patch {
            let column = key.strip_prefix('_').map(str::to_owned).unwrap_or_else(|| {
                let mut snake = String::with_capacity(key.len());
                for (index, ch) in key.chars().enumerate() {
                    if ch.is_ascii_uppercase() && index > 0 {
                        snake.push('_');
                    }
                    snake.extend(ch.to_lowercase());
                }
                snake
            });
            let Some((_, declared_type)) = columns.iter().find(|(name, _)| name == &column) else {
                continue;
            };
            if matches!(
                column.as_str(),
                "id" | "_cvm_run_id" | "_cvm_source_key" | "_cvm_payload"
            ) {
                continue;
            }
            let assignment = if value.is_null() {
                format!("{} = NULL", quote_identifier(&column))
            } else {
                let value = if postgres_type_for_source_column_for_store(declared_type) == "BIGINT"
                    && value.is_boolean()
                {
                    serde_json::Value::from(value.as_bool().unwrap_or(false) as i64)
                } else {
                    value.clone()
                };
                values.push(value);
                let placeholder = format!("${}::jsonb ->> 'value'", values.len() + 3);
                let pg_type =
                    crate::migration::postgres_type_for_source_column_for_store(declared_type);
                format!("{} = ({placeholder})::{pg_type}", quote_identifier(&column))
            };
            assignments.push(assignment);
        }
        if assignments.is_empty() {
            return self
                .latest_materialized_rows("pool_upstream_accounts")
                .await
                .map(|rows| {
                    rows.into_iter().find(|row| {
                        row.get("id").and_then(serde_json::Value::as_i64) == Some(account_id)
                    })
                });
        }
        let query = format!(
            "UPDATE {qualified} AS account SET {} WHERE $1::text IS NOT NULL AND $2::jsonb IS NOT NULL AND account.id=$3 RETURNING to_jsonb(account)",
            assignments.join(", ")
        );
        let patch_json = serde_json::Value::Object(patch.clone());
        let mut request = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(&query)
            .bind(&run_id)
            .bind(sqlx::types::Json(patch_json))
            .bind(account_id);
        for value in values {
            request = request.bind(sqlx::types::Json(serde_json::json!({"value": value})));
        }
        let row = request.fetch_optional(pool).await?;
        Ok(row.map(|value| value.0))
    }

    async fn create_materialized_account(
        &self,
        payload: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        let payload_object = payload
            .as_object()
            .context("upstream account payload must be an object")?;
        let pool = self.database.pool();
        let run_id: Option<String> = sqlx::query_scalar(
            "SELECT run_id FROM cvm_migration.runs WHERE state IN ('seeded','verified','finalized') ORDER BY updated_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .context("failed to select the latest verified migration run for account insert")?;
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let (schema, table): (String, String) = sqlx::query_as(
            "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1 AND object_name='pool_upstream_accounts'",
        )
        .bind(&run_id)
        .fetch_optional(pool)
        .await?
        .context("migrated upstream account object is unavailable")?;
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT column_name, declared_type FROM cvm_migration.object_columns WHERE run_id=$1 AND object_name='pool_upstream_accounts' ORDER BY ordinal",
        )
        .bind(&run_id)
        .fetch_all(pool)
        .await?;
        let id: i64 = if columns.iter().any(|(name, _)| name == "id") {
            let sequence_name: Option<String> =
                sqlx::query_scalar("SELECT pg_get_serial_sequence($1, 'id')")
                    .bind(format!("{schema}.{table}"))
                    .fetch_one(pool)
                    .await?;
            if let Some(sequence_name) = sequence_name {
                sqlx::query_scalar("SELECT nextval($1)")
                    .bind(sequence_name)
                    .fetch_one(pool)
                    .await?
            } else {
                let query = format!(
                    "SELECT COALESCE(MAX(\"id\"), 0) + 1 FROM {qualified} WHERE $1::text IS NOT NULL"
                );
                sqlx::query_scalar(&query)
                    .bind(&run_id)
                    .fetch_one(pool)
                    .await?
            }
        } else {
            0
        };
        let mut full_payload = payload_object.clone();
        if columns.iter().any(|(name, _)| name == "id") {
            full_payload.insert("id".to_owned(), serde_json::Value::from(id));
        }
        let mut target_columns = Vec::new();
        let mut expressions = Vec::new();
        for (column, declared_type) in columns {
            if let Some(value) = full_payload.get(&column) {
                if value.is_null() {
                    continue;
                }
                target_columns.push(quote_identifier(&column));
                let pg_type = postgres_type_for_source_column_for_store(&declared_type);
                expressions.push(format!(
                    "(($1::jsonb ->> {})::{pg_type})",
                    quote_literal_for_store(&column)
                ));
            }
        }
        let query = format!(
            "INSERT INTO {qualified}({}) VALUES ({}) RETURNING to_jsonb({qualified})",
            target_columns.join(", "),
            expressions.join(", ")
        );
        let payload = serde_json::Value::Object(full_payload);
        let row = sqlx::query_scalar::<_, sqlx::types::Json<serde_json::Value>>(&query)
            .bind(sqlx::types::Json(payload))
            .fetch_one(pool)
            .await
            .context("failed to insert PostgreSQL primary upstream account")?;
        Ok(Some(row.0))
    }

    async fn record_proxy_request(&self, record: ProxyRequestRecord) -> Result<()> {
        sqlx::query(
            "INSERT INTO cvm_activity.proxy_requests(invoke_id,method,request_path,status_code,request_bytes,response_bytes,duration_ms,error) VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(invoke_id) DO NOTHING",
        )
        .bind(record.invoke_id)
        .bind(record.method)
        .bind(record.request_path)
        .bind(record.status_code.map(i32::from))
        .bind(record.request_bytes as i64)
        .bind(record.response_bytes as i64)
        .bind(record.duration_ms)
        .bind(record.error)
        .execute(self.database.pool())
        .await
        .context("failed to persist PostgreSQL primary proxy request")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) enum PrimaryStateStoreBackend {
    Sqlite(SqlitePrimaryStateStore),
    Postgres(PostgresPrimaryStateStore),
}

pub(crate) async fn open_runtime_store(
    config: &AppConfig,
    sqlite_pool: Option<Pool<Sqlite>>,
) -> Result<Arc<dyn PrimaryStateStore>> {
    match config.primary_database_backend {
        PrimaryDatabaseBackend::Sqlite => {
            let pool = sqlite_pool.context("SQLite primary state store requires a SQLite pool")?;
            let store = SqlitePrimaryStateStore::new(pool);
            store.ensure_schema().await?;
            Ok(Arc::new(store))
        }
        PrimaryDatabaseBackend::Postgres => {
            let database = PrimaryDatabase::connect_runtime(config).await?;
            Ok(Arc::new(PostgresPrimaryStateStore::new(database)))
        }
    }
}

#[async_trait::async_trait]
impl PrimaryStateStore for PrimaryStateStoreBackend {
    async fn get_json(&self, key: &str) -> Result<Option<serde_json::Value>> {
        match self {
            Self::Sqlite(store) => store.get_json(key).await,
            Self::Postgres(store) => store.get_json(key).await,
        }
    }

    async fn put_json(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        match self {
            Self::Sqlite(store) => store.put_json(key, value).await,
            Self::Postgres(store) => store.put_json(key, value).await,
        }
    }

    async fn put_json_batch(&self, values: &[(&str, serde_json::Value)]) -> Result<()> {
        match self {
            Self::Sqlite(store) => store.put_json_batch(values).await,
            Self::Postgres(store) => store.put_json_batch(values).await,
        }
    }

    async fn append_json_array(
        &self,
        key: &str,
        value: serde_json::Value,
        max_len: usize,
    ) -> Result<()> {
        match self {
            Self::Sqlite(store) => store.append_json_array(key, value, max_len).await,
            Self::Postgres(store) => store.append_json_array(key, value, max_len).await,
        }
    }

    async fn latest_materialized_rows(&self, object_name: &str) -> Result<Vec<serde_json::Value>> {
        match self {
            Self::Sqlite(store) => store.latest_materialized_rows(object_name).await,
            Self::Postgres(store) => store.latest_materialized_rows(object_name).await,
        }
    }

    async fn delete_materialized_rows_by_source_keys(
        &self,
        object_name: &str,
        source_keys: &[String],
    ) -> Result<u64> {
        match self {
            Self::Sqlite(store) => {
                store
                    .delete_materialized_rows_by_source_keys(object_name, source_keys)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .delete_materialized_rows_by_source_keys(object_name, source_keys)
                    .await
            }
        }
    }

    async fn update_materialized_text_fields(
        &self,
        object_name: &str,
        source_key: &str,
        fields: &[(&str, &str)],
    ) -> Result<()> {
        match self {
            Self::Sqlite(store) => {
                store
                    .update_materialized_text_fields(object_name, source_key, fields)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .update_materialized_text_fields(object_name, source_key, fields)
                    .await
            }
        }
    }

    async fn update_materialized_integer_fields(
        &self,
        object_name: &str,
        source_key: &str,
        fields: &[(&str, i64)],
    ) -> Result<()> {
        match self {
            Self::Sqlite(store) => {
                store
                    .update_materialized_integer_fields(object_name, source_key, fields)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .update_materialized_integer_fields(object_name, source_key, fields)
                    .await
            }
        }
    }

    async fn update_materialized_float_fields(
        &self,
        object_name: &str,
        source_key: &str,
        fields: &[(&str, f64)],
    ) -> Result<()> {
        match self {
            Self::Sqlite(store) => {
                store
                    .update_materialized_float_fields(object_name, source_key, fields)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .update_materialized_float_fields(object_name, source_key, fields)
                    .await
            }
        }
    }

    async fn mutate_materialized_account(
        &self,
        account_id: i64,
        action: &str,
        group_name: Option<&str>,
    ) -> Result<Option<serde_json::Value>> {
        match self {
            Self::Sqlite(store) => {
                store
                    .mutate_materialized_account(account_id, action, group_name)
                    .await
            }
            Self::Postgres(store) => {
                store
                    .mutate_materialized_account(account_id, action, group_name)
                    .await
            }
        }
    }

    async fn update_materialized_account(
        &self,
        account_id: i64,
        patch: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        match self {
            Self::Sqlite(store) => store.update_materialized_account(account_id, patch).await,
            Self::Postgres(store) => store.update_materialized_account(account_id, patch).await,
        }
    }

    async fn create_materialized_account(
        &self,
        payload: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>> {
        match self {
            Self::Sqlite(store) => store.create_materialized_account(payload).await,
            Self::Postgres(store) => store.create_materialized_account(payload).await,
        }
    }

    async fn record_proxy_request(&self, record: ProxyRequestRecord) -> Result<()> {
        match self {
            Self::Sqlite(store) => store.record_proxy_request(record).await,
            Self::Postgres(store) => store.record_proxy_request(record).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sqlite_state_store_round_trips_json_idempotently() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        assert_eq!(store.get_json("missing").await.unwrap(), None);
        let value = json!({"revision": 3, "enabled": true});
        store.put_json("runtime", &value).await.unwrap();
        assert_eq!(
            store.get_json("runtime").await.unwrap(),
            Some(value.clone())
        );
        store
            .put_json("runtime", &json!({"revision": 4}))
            .await
            .unwrap();
        assert_eq!(
            store.get_json("runtime").await.unwrap(),
            Some(json!({"revision": 4}))
        );
    }

    #[tokio::test]
    async fn sqlite_state_store_commits_json_batch_as_one_unit() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        store
            .put_json_batch(&[
                ("catalog", json!({"version": 1})),
                ("quotes", json!([1, 2])),
            ])
            .await
            .unwrap();
        assert_eq!(
            store.get_json("catalog").await.unwrap(),
            Some(json!({"version": 1}))
        );
        assert_eq!(store.get_json("quotes").await.unwrap(), Some(json!([1, 2])));
    }

    #[tokio::test]
    async fn sqlite_state_store_appends_json_array_with_bounded_history() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        for value in 0..4 {
            store
                .append_json_array("queue", json!({"value": value}), 2)
                .await
                .unwrap();
        }
        assert_eq!(
            store.get_json("queue").await.unwrap(),
            Some(json!([{"value": 2}, {"value": 3}]))
        );
    }
}
