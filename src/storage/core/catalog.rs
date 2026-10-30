use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ModuleId {
    TrafficArchive,
    Analytics,
    AccountRouting,
    TaskOperations,
    Performance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RecoveryObligation {
    TrafficFact,
    ArchivePublication,
    TrafficRecovery,
    AnalyticsProjection,
    AccountControl,
    TaskOperations,
    PerformanceInventory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableOwner {
    pub(crate) table: String,
    pub(crate) module: ModuleId,
    pub(crate) transaction_group: &'static str,
    pub(crate) recovery_obligation: RecoveryObligation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaObject {
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) table: String,
    pub(crate) definition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForeignKeyRelation {
    pub(crate) table: String,
    pub(crate) referenced_table: String,
    pub(crate) columns: Vec<String>,
    pub(crate) referenced_columns: Vec<String>,
    pub(crate) on_delete: String,
    pub(crate) on_update: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TriggerWriteRelation {
    pub(crate) trigger: String,
    pub(crate) source_table: String,
    pub(crate) target_table: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NativeColumnType {
    BigInt,
    DoublePrecision,
    Boolean,
    Text,
    Bytea,
}

impl NativeColumnType {
    pub(crate) fn postgres_name(self) -> &'static str {
        match self {
            Self::BigInt => "BIGINT",
            Self::DoublePrecision => "DOUBLE PRECISION",
            Self::Boolean => "BOOLEAN",
            Self::Text => "TEXT",
            Self::Bytea => "BYTEA",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeColumn {
    pub(crate) name: String,
    pub(crate) ordinal: i64,
    pub(crate) declared_type: String,
    pub(crate) postgres_type: NativeColumnType,
    pub(crate) nullable: bool,
    pub(crate) default_sql: Option<String>,
    pub(crate) primary_key_ordinal: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeUniqueConstraint {
    pub(crate) name: String,
    pub(crate) columns: Vec<String>,
    pub(crate) partial_predicate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeIndex {
    pub(crate) name: String,
    pub(crate) columns: Vec<String>,
    pub(crate) unique: bool,
    pub(crate) partial_predicate: Option<String>,
    pub(crate) expression: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NativeResourceReferenceKind {
    Raw,
    ArchiveFile,
    StagedFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeResourceReference {
    pub(crate) column: String,
    pub(crate) kind: NativeResourceReferenceKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeCaptureDeleteSemantics {
    pub(crate) operation_column: String,
    pub(crate) delete_value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeCaptureMetadata {
    // These columns belong to the capture log/state tables, not to each business table copied by
    // NativeRow. Keeping this mapping explicit prevents a virtual source_version column from
    // leaking into PostgreSQL table DDL or wide-row payloads.
    pub(crate) log_table: String,
    pub(crate) state_table: String,
    pub(crate) table_name_column: String,
    pub(crate) stable_key_column: String,
    pub(crate) operation_column: String,
    pub(crate) source_identity_column: String,
    pub(crate) source_version_column: String,
    pub(crate) generation_column: String,
    pub(crate) delete: NativeCaptureDeleteSemantics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeSourceWatermark {
    pub(crate) business_source_version_column: Option<String>,
    pub(crate) stable_key_columns: Vec<String>,
    pub(crate) capture: NativeCaptureMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeTableSpec {
    pub(crate) owner: TableOwner,
    pub(crate) columns: Vec<NativeColumn>,
    pub(crate) primary_key: Vec<String>,
    pub(crate) unique_constraints: Vec<NativeUniqueConstraint>,
    pub(crate) indexes: Vec<NativeIndex>,
    pub(crate) foreign_keys: Vec<ForeignKeyRelation>,
    pub(crate) checks: Vec<String>,
    pub(crate) resource_references: Vec<NativeResourceReference>,
    pub(crate) source_watermark: NativeSourceWatermark,
}

impl NativeTableSpec {
    pub(crate) fn table_name(&self) -> &str {
        &self.owner.table
    }

    pub(crate) fn stable_key_columns(&self) -> &[String] {
        &self.source_watermark.stable_key_columns
    }

    pub(crate) fn postgres_create_table_sql(&self) -> Result<String, CatalogError> {
        let mut definitions = Vec::with_capacity(self.columns.len() + 2);
        for column in &self.columns {
            let mut definition = format!(
                "{} {}",
                quote_native_identifier(&column.name),
                column.postgres_type.postgres_name()
            );
            if !column.nullable {
                definition.push_str(" NOT NULL");
            }
            if let Some(default_sql) = &column.default_sql {
                definition.push_str(" DEFAULT ");
                definition.push_str(&normalize_native_default(default_sql).ok_or_else(|| {
                    CatalogError::NativeCatalogIncomplete {
                        table: self.owner.table.clone(),
                        detail: format!(
                            "unsupported default for column {}{}",
                            column.name,
                            sqlite_expression_detail(default_sql)
                                .map_or_else(String::new, |expression| {
                                    format!(" (SQLite-only expression {expression})")
                                })
                        ),
                    }
                })?);
            }
            definitions.push(definition);
        }
        if !self.primary_key.is_empty() {
            definitions.push(format!(
                "PRIMARY KEY ({})",
                self.primary_key
                    .iter()
                    .map(|column| quote_native_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        for unique in &self.unique_constraints {
            if unique.partial_predicate.is_none() && unique.columns != self.primary_key {
                definitions.push(format!(
                    "CONSTRAINT {} UNIQUE ({})",
                    quote_native_identifier(&unique.name),
                    unique
                        .columns
                        .iter()
                        .map(|column| quote_native_identifier(column))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        for check in &self.checks {
            let check = normalize_native_check(&self.owner.table, check)?;
            definitions.push(format!("CHECK ({check})"));
        }
        Ok(format!(
            "CREATE TABLE IF NOT EXISTS {} (\n    {}\n)",
            quote_native_identifier(&self.owner.table),
            definitions.join(",\n    ")
        ))
    }

    pub(crate) fn postgres_foreign_key_sql(&self) -> Result<Vec<String>, CatalogError> {
        let mut statements = Vec::new();
        for (index, foreign_key) in self.foreign_keys.iter().enumerate() {
            if foreign_key.columns.is_empty() || foreign_key.referenced_columns.is_empty() {
                return Err(CatalogError::NativeCatalogIncomplete {
                    table: self.owner.table.clone(),
                    detail: format!("foreign key {index} has incomplete column metadata"),
                });
            }
            let constraint_name = format!("fk_{}_{}", self.owner.table, index);
            let source_table_regclass =
                quote_native_literal(&format!("\"{}\"", self.owner.table.replace('"', "\"\"")));
            let referenced_table_regclass = quote_native_literal(&format!(
                "\"{}\"",
                foreign_key.referenced_table.replace('"', "\"\"")
            ));
            let source_columns = native_text_array(&foreign_key.columns);
            let referenced_columns = native_text_array(&foreign_key.referenced_columns);
            let update_action = normalize_fk_action(&foreign_key.on_update);
            let delete_action = normalize_fk_action(&foreign_key.on_delete);
            statements.push(format!(
                "DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint AS pc
         WHERE pc.contype = 'f'
           AND pc.conrelid = to_regclass({source_table_regclass})
           AND pc.confrelid = to_regclass({referenced_table_regclass})
           AND pc.conkey = ARRAY(
               SELECT attribute.attnum
                 FROM pg_attribute AS attribute
                WHERE attribute.attrelid = pc.conrelid
                  AND attribute.attname = ANY ({source_columns})
                ORDER BY array_position({source_columns}, attribute.attname)
           )
           AND pc.confkey = ARRAY(
               SELECT attribute.attnum
                 FROM pg_attribute AS attribute
                WHERE attribute.attrelid = pc.confrelid
                  AND attribute.attname = ANY ({referenced_columns})
                ORDER BY array_position({referenced_columns}, attribute.attname)
           )
           AND pc.confupdtype = '{update_code}'
           AND pc.confdeltype = '{delete_code}'
    ) THEN
        ALTER TABLE {source_table} ADD CONSTRAINT {constraint_name} FOREIGN KEY ({source_columns_sql}) REFERENCES {referenced_table} ({referenced_columns_sql}) ON UPDATE {update_action} ON DELETE {delete_action};
    END IF;
END
$$",
                source_table_regclass = source_table_regclass,
                referenced_table_regclass = referenced_table_regclass,
                source_columns = source_columns,
                referenced_columns = referenced_columns,
                update_code = postgres_fk_action_code(update_action),
                delete_code = postgres_fk_action_code(delete_action),
                source_table = quote_native_identifier(&self.owner.table),
                constraint_name = quote_native_identifier(&constraint_name),
                source_columns_sql = foreign_key
                    .columns
                    .iter()
                    .map(|column| quote_native_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", "),
                referenced_table = quote_native_identifier(&foreign_key.referenced_table),
                referenced_columns_sql = foreign_key
                    .referenced_columns
                    .iter()
                    .map(|column| quote_native_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        Ok(statements)
    }

    pub(crate) fn postgres_index_sql(&self) -> Result<Vec<String>, CatalogError> {
        let mut statements = Vec::new();
        for index in &self.indexes {
            if index.name.starts_with("sqlite_autoindex_") {
                continue;
            }
            if index.expression {
                if index.unique {
                    return Err(CatalogError::NativeCatalogIncomplete {
                        table: self.owner.table.clone(),
                        detail: format!(
                            "unique expression index {} cannot be represented safely",
                            index.name
                        ),
                    });
                }
                continue;
            }
            if index.columns.is_empty() {
                return Err(CatalogError::NativeCatalogIncomplete {
                    table: self.owner.table.clone(),
                    detail: format!("index {} has no column metadata", index.name),
                });
            }
            let unique = if index.unique { "UNIQUE " } else { "" };
            let predicate = if let Some(value) = index.partial_predicate.as_deref() {
                let value =
                    normalize_native_index_predicate(&self.owner.table, &index.name, value)?;
                format!(" WHERE {value}")
            } else {
                String::new()
            };
            statements.push(format!(
                "CREATE {unique}INDEX IF NOT EXISTS {} ON {} ({}){predicate}",
                quote_native_identifier(&index.name),
                quote_native_identifier(&self.owner.table),
                index
                    .columns
                    .iter()
                    .map(|column| quote_native_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(statements)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeMigrationCatalog {
    pub(crate) modules: BTreeSet<ModuleId>,
    pub(crate) tables: Vec<NativeTableSpec>,
    pub(crate) trigger_writes: Vec<TriggerWriteRelation>,
}

impl NativeMigrationCatalog {
    pub(crate) fn table(&self, table: &str) -> Option<&NativeTableSpec> {
        self.tables.iter().find(|spec| spec.table_name() == table)
    }

    pub(crate) fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.iter().map(NativeTableSpec::table_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CatalogError {
    UnknownOwnership(String),
    CrossModuleTransaction {
        first: ModuleId,
        other: ModuleId,
        table: String,
    },
    CrossModuleForeignKey {
        table: String,
        referenced_table: String,
    },
    NativeCatalogIncomplete {
        table: String,
        detail: String,
    },
    NativeCrossModuleEdge {
        edge_kind: &'static str,
        source_table: String,
        target_table: String,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOwnership(table) => {
                write!(formatter, "unknown SQLite table ownership: {table}")
            }
            Self::CrossModuleTransaction {
                first,
                other,
                table,
            } => write!(
                formatter,
                "transaction crosses module boundary from {first:?} to {other:?} at {table}"
            ),
            Self::CrossModuleForeignKey {
                table,
                referenced_table,
            } => write!(
                formatter,
                "foreign key crosses module boundary from {table} to {referenced_table}"
            ),
            Self::NativeCatalogIncomplete { table, detail } => {
                write!(
                    formatter,
                    "native catalog cannot safely migrate {table}: {detail}"
                )
            }
            Self::NativeCrossModuleEdge {
                edge_kind,
                source_table,
                target_table,
            } => write!(
                formatter,
                "native migration closure crosses an unapproved {edge_kind} edge from {source_table} to {target_table}"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleCatalog {
    owners: Vec<TableOwner>,
    excluded_tables: BTreeSet<String>,
    objects: Vec<SchemaObject>,
    foreign_keys: Vec<ForeignKeyRelation>,
    trigger_writes: Vec<TriggerWriteRelation>,
    table_schemas: BTreeMap<String, NativeTableSpec>,
}

impl ModuleCatalog {
    pub(crate) async fn inspect_sqlite(pool: &SqlitePool) -> Result<Self, CatalogError> {
        let object_rows = sqlx::query(
            "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE (type = 'table' AND name NOT LIKE 'sqlite_%') OR type IN ('index', 'trigger') ORDER BY type, name",
        )
        .fetch_all(pool)
        .await
        .map_err(|_| CatalogError::UnknownOwnership("<sqlite_master unavailable>".into()))?;
        let mut owners = Vec::new();
        let mut excluded_tables = BTreeSet::new();
        let mut objects = Vec::with_capacity(object_rows.len());
        for row in object_rows {
            let kind = row.try_get::<String, _>("type").map_err(|_| {
                CatalogError::UnknownOwnership("<invalid schema object type>".into())
            })?;
            let name = row.try_get::<String, _>("name").map_err(|_| {
                CatalogError::UnknownOwnership("<invalid schema object name>".into())
            })?;
            let table = row.try_get::<String, _>("tbl_name").map_err(|_| {
                CatalogError::UnknownOwnership("<invalid schema object owner>".into())
            })?;
            let definition = row.try_get::<Option<String>, _>("sql").map_err(|_| {
                CatalogError::UnknownOwnership("<invalid schema object definition>".into())
            })?;
            if kind == "table" {
                if is_migration_metadata_table(&name) {
                    excluded_tables.insert(name.clone());
                } else {
                    owners.push(classify_table(name.clone())?);
                }
            }
            objects.push(SchemaObject {
                kind,
                name,
                table,
                definition,
            });
        }
        let mut foreign_keys = Vec::new();
        for owner in &owners {
            let escaped_name = owner.table.replace('\'', "''");
            let query = format!("PRAGMA foreign_key_list('{escaped_name}')");
            let mut grouped = BTreeMap::<i64, ForeignKeyRelation>::new();
            for row in sqlx::query(&query).fetch_all(pool).await.map_err(|_| {
                CatalogError::UnknownOwnership(format!(
                    "<foreign keys for {} unavailable>",
                    owner.table
                ))
            })? {
                let id = row.try_get::<i64, _>("id").map_err(|_| {
                    CatalogError::UnknownOwnership(format!(
                        "<invalid foreign key id for {}>",
                        owner.table
                    ))
                })?;
                let referenced_table = row.try_get::<String, _>("table").map_err(|_| {
                    CatalogError::UnknownOwnership(format!(
                        "<invalid foreign key target for {}>",
                        owner.table
                    ))
                })?;
                let relation = grouped.entry(id).or_insert_with(|| ForeignKeyRelation {
                    table: owner.table.clone(),
                    referenced_table: referenced_table.clone(),
                    columns: Vec::new(),
                    referenced_columns: Vec::new(),
                    on_delete: row
                        .try_get::<String, _>("on_delete")
                        .unwrap_or_else(|_| "NO ACTION".into()),
                    on_update: row
                        .try_get::<String, _>("on_update")
                        .unwrap_or_else(|_| "NO ACTION".into()),
                });
                if relation.referenced_table != referenced_table {
                    return Err(CatalogError::NativeCatalogIncomplete {
                        table: owner.table.clone(),
                        detail: format!("foreign key {id} changes referenced table"),
                    });
                }
                let source_column = row.try_get::<Option<String>, _>("from").map_err(|_| {
                    CatalogError::NativeCatalogIncomplete {
                        table: owner.table.clone(),
                        detail: format!("foreign key {id} has no source column"),
                    }
                })?;
                let referenced_column = row.try_get::<Option<String>, _>("to").map_err(|_| {
                    CatalogError::NativeCatalogIncomplete {
                        table: owner.table.clone(),
                        detail: format!("foreign key {id} has no target column"),
                    }
                })?;
                if let Some(source_column) = source_column {
                    relation.columns.push(source_column);
                }
                if let Some(referenced_column) = referenced_column {
                    relation.referenced_columns.push(referenced_column);
                }
            }
            foreign_keys.extend(grouped.into_values());
        }
        let mut trigger_writes = Vec::new();
        for object in objects.iter().filter(|object| object.kind == "trigger") {
            let definition = object.definition.as_deref().ok_or_else(|| {
                CatalogError::UnknownOwnership(format!(
                    "<trigger definition for {} unavailable>",
                    object.name
                ))
            })?;
            if is_migration_metadata_table(&object.table) {
                continue;
            }
            for target_table in trigger_write_tables(definition) {
                if is_migration_metadata_table(&target_table) {
                    continue;
                }
                if !owners.iter().any(|owner| owner.table == target_table) {
                    return Err(CatalogError::UnknownOwnership(target_table));
                }
                trigger_writes.push(TriggerWriteRelation {
                    trigger: object.name.clone(),
                    source_table: object.table.clone(),
                    target_table,
                });
            }
        }
        let table_schemas = inspect_table_schemas(pool, &owners, &objects, &foreign_keys).await?;
        let catalog = Self {
            owners,
            excluded_tables,
            objects,
            foreign_keys,
            trigger_writes,
            table_schemas,
        };
        catalog.validate_schema_object_owners()?;
        catalog.validate_foreign_key_ownership()?;
        Ok(catalog)
    }

    pub(crate) fn owners(&self) -> &[TableOwner] {
        &self.owners
    }

    pub(crate) fn excluded_tables(&self) -> &BTreeSet<String> {
        &self.excluded_tables
    }

    pub(crate) fn schema_objects(&self) -> &[SchemaObject] {
        &self.objects
    }

    pub(crate) fn foreign_keys(&self) -> &[ForeignKeyRelation] {
        &self.foreign_keys
    }

    pub(crate) fn trigger_writes(&self) -> &[TriggerWriteRelation] {
        &self.trigger_writes
    }

    pub(crate) fn table_schemas(&self) -> impl Iterator<Item = &NativeTableSpec> {
        self.table_schemas.values()
    }

    /// Build the exact native table closure for one approved migration unit.
    ///
    /// Both sides of a trigger/FK edge are considered. This catches incoming writers as well as
    /// the more obvious outgoing writes, so a target cannot be prepared while an unselected
    /// SQLite owner can still mutate it.
    pub(crate) fn native_migration_catalog(
        &self,
        modules: impl IntoIterator<Item = ModuleId>,
    ) -> Result<NativeMigrationCatalog, CatalogError> {
        let modules = modules.into_iter().collect::<BTreeSet<_>>();
        if modules.is_empty() {
            return Err(CatalogError::NativeCatalogIncomplete {
                table: "<module set>".into(),
                detail: "native migration requires at least one module".into(),
            });
        }
        let mut closure = self
            .owners
            .iter()
            .filter(|owner| modules.contains(&owner.module))
            .map(|owner| owner.table.clone())
            .collect::<BTreeSet<_>>();
        if closure.is_empty() {
            return Err(CatalogError::NativeCatalogIncomplete {
                table: "<module set>".into(),
                detail: "approved modules have no catalog tables".into(),
            });
        }

        let owner_module = |table: &str| {
            self.owners
                .iter()
                .find(|owner| owner.table == table)
                .map(|owner| owner.module)
                .ok_or_else(|| CatalogError::UnknownOwnership(table.into()))
        };
        let mut changed = true;
        while changed {
            changed = false;
            for relation in &self.trigger_writes {
                if closure.contains(&relation.source_table)
                    || closure.contains(&relation.target_table)
                {
                    let source_module = owner_module(&relation.source_table)?;
                    let target_module = owner_module(&relation.target_table)?;
                    if !modules.contains(&source_module) || !modules.contains(&target_module) {
                        return Err(CatalogError::NativeCrossModuleEdge {
                            edge_kind: "trigger",
                            source_table: relation.source_table.clone(),
                            target_table: relation.target_table.clone(),
                        });
                    }
                    changed |= closure.insert(relation.source_table.clone());
                    changed |= closure.insert(relation.target_table.clone());
                }
            }
            for relation in &self.foreign_keys {
                if closure.contains(&relation.table) || closure.contains(&relation.referenced_table)
                {
                    let source_module = owner_module(&relation.table)?;
                    let target_module = owner_module(&relation.referenced_table)?;
                    if !modules.contains(&source_module) || !modules.contains(&target_module) {
                        return Err(CatalogError::NativeCrossModuleEdge {
                            edge_kind: "foreign-key",
                            source_table: relation.table.clone(),
                            target_table: relation.referenced_table.clone(),
                        });
                    }
                    changed |= closure.insert(relation.table.clone());
                    changed |= closure.insert(relation.referenced_table.clone());
                }
            }
        }

        let mut tables = Vec::with_capacity(closure.len());
        for table in &closure {
            let spec = self.table_schemas.get(table).ok_or_else(|| {
                CatalogError::NativeCatalogIncomplete {
                    table: table.clone(),
                    detail: "SQLite table schema was not captured by the executable catalog".into(),
                }
            })?;
            if spec.stable_key_columns().is_empty() {
                return Err(CatalogError::NativeCatalogIncomplete {
                    table: table.clone(),
                    detail: "table has no deterministic primary or non-null unique key".into(),
                });
            }
            tables.push(spec.clone());
        }
        tables.sort_by(|left, right| left.table_name().cmp(right.table_name()));
        let trigger_writes = self
            .trigger_writes
            .iter()
            .filter(|relation| {
                closure_contains_table(&tables, &relation.source_table)
                    && closure_contains_table(&tables, &relation.target_table)
            })
            .cloned()
            .collect();
        Ok(NativeMigrationCatalog {
            modules,
            tables,
            trigger_writes,
        })
    }

    pub(crate) fn cross_module_trigger_writes(&self) -> Vec<&TriggerWriteRelation> {
        self.trigger_writes
            .iter()
            .filter(|relation| {
                let source = self
                    .owners
                    .iter()
                    .find(|owner| owner.table == relation.source_table)
                    .map(|owner| owner.module);
                let target = self
                    .owners
                    .iter()
                    .find(|owner| owner.table == relation.target_table)
                    .map(|owner| owner.module);
                source != target
            })
            .collect()
    }

    /// Validate a transaction after expanding trigger writes and foreign-key endpoints.
    pub(crate) fn validate_transaction<'a>(
        &self,
        tables: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), CatalogError> {
        let mut closure = BTreeSet::new();
        let mut pending = Vec::new();
        for table in tables {
            if !self.owners.iter().any(|owner| owner.table == table) {
                return Err(CatalogError::UnknownOwnership(table.into()));
            }
            if closure.insert(table.to_string()) {
                pending.push(table.to_string());
            }
        }
        while let Some(source_table) = pending.pop() {
            for relation in self
                .trigger_writes
                .iter()
                .filter(|relation| relation.source_table == source_table)
            {
                if closure.insert(relation.target_table.clone()) {
                    pending.push(relation.target_table.clone());
                }
            }
            for relation in &self.foreign_keys {
                let related_table = if relation.table == source_table {
                    Some(&relation.referenced_table)
                } else if relation.referenced_table == source_table {
                    Some(&relation.table)
                } else {
                    None
                };
                if let Some(related_table) = related_table
                    && closure.insert(related_table.clone())
                {
                    pending.push(related_table.clone());
                }
            }
        }

        let mut module = None;
        for table in closure {
            let owner = self
                .owners
                .iter()
                .find(|owner| owner.table == table)
                .map(|owner| owner.module)
                .ok_or_else(|| CatalogError::UnknownOwnership(table.clone()))?;
            if let Some(first) = module {
                if first != owner {
                    return Err(CatalogError::CrossModuleTransaction {
                        first,
                        other: owner,
                        table,
                    });
                }
            } else {
                module = Some(owner);
            }
        }
        Ok(())
    }

    fn validate_foreign_key_ownership(&self) -> Result<(), CatalogError> {
        for edge in &self.foreign_keys {
            let source = self
                .owners
                .iter()
                .find(|owner| owner.table == edge.table)
                .ok_or_else(|| CatalogError::UnknownOwnership(edge.table.clone()))?;
            let target = self
                .owners
                .iter()
                .find(|owner| owner.table == edge.referenced_table)
                .ok_or_else(|| CatalogError::UnknownOwnership(edge.referenced_table.clone()))?;
            if source.module != target.module {
                return Err(CatalogError::CrossModuleForeignKey {
                    table: edge.table.clone(),
                    referenced_table: edge.referenced_table.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_schema_object_owners(&self) -> Result<(), CatalogError> {
        for object in self.objects.iter().filter(|object| object.kind != "table") {
            if self.excluded_tables.contains(&object.table) {
                continue;
            }
            if !self.owners.iter().any(|owner| owner.table == object.table) {
                return Err(CatalogError::UnknownOwnership(object.table.clone()));
            }
        }
        Ok(())
    }

    pub(crate) fn modules(&self) -> BTreeSet<ModuleId> {
        self.owners.iter().map(|owner| owner.module).collect()
    }
}

// Durable migration capture protocol tables are inspected as schema objects but excluded from
// business ownership and native module closure.
const MIGRATION_METADATA_TABLES: &[&str] = &["migration_change_log", "migration_change_log_state"];

fn is_migration_metadata_table(table: &str) -> bool {
    MIGRATION_METADATA_TABLES.contains(&table)
}

fn classify_table(table: String) -> Result<TableOwner, CatalogError> {
    let (module, transaction_group, recovery_obligation) = if traffic_table(&table) {
        let transaction_group = traffic_group(&table)?;
        let recovery_obligation = match transaction_group {
            "traffic_fact" => RecoveryObligation::TrafficFact,
            "archive_publication" => RecoveryObligation::ArchivePublication,
            "traffic_recovery_obligation" => RecoveryObligation::TrafficRecovery,
            _ => return Err(CatalogError::UnknownOwnership(table)),
        };
        (
            ModuleId::TrafficArchive,
            transaction_group,
            recovery_obligation,
        )
    } else if analytics_table(&table) {
        (
            ModuleId::Analytics,
            "analytics_projection",
            RecoveryObligation::AnalyticsProjection,
        )
    } else if account_table(&table) {
        (
            ModuleId::AccountRouting,
            "account_control",
            RecoveryObligation::AccountControl,
        )
    } else if task_table(&table) {
        (
            ModuleId::TaskOperations,
            "task_operations",
            RecoveryObligation::TaskOperations,
        )
    } else if performance_table(&table) {
        (
            ModuleId::Performance,
            "performance",
            RecoveryObligation::PerformanceInventory,
        )
    } else {
        return Err(CatalogError::UnknownOwnership(table));
    };
    Ok(TableOwner {
        table,
        module,
        transaction_group,
        recovery_obligation,
    })
}

async fn inspect_table_schemas(
    pool: &SqlitePool,
    owners: &[TableOwner],
    objects: &[SchemaObject],
    foreign_keys: &[ForeignKeyRelation],
) -> Result<BTreeMap<String, NativeTableSpec>, CatalogError> {
    let mut schemas = BTreeMap::new();
    for owner in owners {
        let escaped_name = owner.table.replace('\'', "''");
        let columns_query = format!("PRAGMA table_xinfo('{escaped_name}')");
        let columns = sqlx::query(&columns_query)
            .fetch_all(pool)
            .await
            .map_err(|_| CatalogError::NativeCatalogIncomplete {
                table: owner.table.clone(),
                detail: "SQLite column metadata is unavailable".into(),
            })?
            .into_iter()
            .map(|row| {
                let ordinal = row.try_get::<i64, _>("cid").map_err(|_| {
                    CatalogError::NativeCatalogIncomplete {
                        table: owner.table.clone(),
                        detail: "SQLite column ordinal is invalid".into(),
                    }
                })?;
                let name = row.try_get::<String, _>("name").map_err(|_| {
                    CatalogError::NativeCatalogIncomplete {
                        table: owner.table.clone(),
                        detail: "SQLite column name is invalid".into(),
                    }
                })?;
                let declared_type = row.try_get::<String, _>("type").unwrap_or_default();
                let primary_key_ordinal = match row.try_get::<i64, _>("pk") {
                    Ok(value) if value > 0 => Some(value),
                    Ok(_) => None,
                    Err(_) => {
                        return Err(CatalogError::NativeCatalogIncomplete {
                            table: owner.table.clone(),
                            detail: format!("SQLite primary-key metadata is invalid for {name}"),
                        });
                    }
                };
                Ok(NativeColumn {
                    name,
                    ordinal,
                    postgres_type: native_column_type(&declared_type),
                    declared_type,
                    nullable: row.try_get::<i64, _>("notnull").unwrap_or(0) == 0,
                    default_sql: row.try_get("dflt_value").ok().flatten(),
                    primary_key_ordinal,
                })
            })
            .collect::<Result<Vec<_>, CatalogError>>()?;
        if columns.is_empty() {
            return Err(CatalogError::NativeCatalogIncomplete {
                table: owner.table.clone(),
                detail: "SQLite table has no columns".into(),
            });
        }
        let mut primary_key = columns
            .iter()
            .filter_map(|column| {
                column
                    .primary_key_ordinal
                    .map(|ordinal| (ordinal, column.name.clone()))
            })
            .collect::<Vec<_>>();
        primary_key.sort_by_key(|(ordinal, _)| *ordinal);
        let primary_key = primary_key
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>();
        let indexes = inspect_indexes(pool, owner, objects).await?;
        let unique_constraints = indexes
            .iter()
            .filter(|index| index.unique && !index.expression)
            .map(|index| NativeUniqueConstraint {
                name: index.name.clone(),
                columns: index.columns.clone(),
                partial_predicate: index.partial_predicate.clone(),
            })
            .collect::<Vec<_>>();
        let stable_key_columns = if !primary_key.is_empty() {
            primary_key.clone()
        } else {
            unique_constraints
                .iter()
                .find(|constraint| {
                    constraint.partial_predicate.is_none()
                        && constraint.columns.iter().all(|name| {
                            columns
                                .iter()
                                .find(|column| &column.name == name)
                                .is_some_and(|column| !column.nullable)
                        })
                })
                .map(|constraint| constraint.columns.clone())
                .unwrap_or_default()
        };
        let resource_references = columns
            .iter()
            .filter_map(|column| {
                native_resource_reference(&column.name).map(|kind| NativeResourceReference {
                    column: column.name.clone(),
                    kind,
                })
            })
            .collect();
        let table_definition = objects
            .iter()
            .find(|object| object.kind == "table" && object.table == owner.table)
            .and_then(|object| object.definition.clone());
        let checks = extract_check_constraints(table_definition.as_deref());
        let business_source_version_column = columns
            .iter()
            .find(|column| column.name == "source_version")
            .map(|column| column.name.clone());
        schemas.insert(
            owner.table.clone(),
            NativeTableSpec {
                owner: owner.clone(),
                columns,
                primary_key,
                unique_constraints,
                indexes,
                foreign_keys: foreign_keys
                    .iter()
                    .filter(|foreign_key| foreign_key.table == owner.table)
                    .cloned()
                    .collect(),
                checks,
                resource_references,
                source_watermark: NativeSourceWatermark {
                    business_source_version_column,
                    stable_key_columns,
                    capture: native_capture_metadata(),
                },
            },
        );
    }
    Ok(schemas)
}

async fn inspect_indexes(
    pool: &SqlitePool,
    owner: &TableOwner,
    objects: &[SchemaObject],
) -> Result<Vec<NativeIndex>, CatalogError> {
    let escaped_name = owner.table.replace('\'', "''");
    let query = format!("PRAGMA index_list('{escaped_name}')");
    let mut indexes = Vec::new();
    for row in sqlx::query(&query).fetch_all(pool).await.map_err(|_| {
        CatalogError::NativeCatalogIncomplete {
            table: owner.table.clone(),
            detail: "SQLite index metadata is unavailable".into(),
        }
    })? {
        let name = row.try_get::<String, _>("name").map_err(|_| {
            CatalogError::NativeCatalogIncomplete {
                table: owner.table.clone(),
                detail: "SQLite index name is invalid".into(),
            }
        })?;
        let index_query = format!("PRAGMA index_info('{}')", name.replace('\'', "''"));
        let index_rows = sqlx::query(&index_query)
            .fetch_all(pool)
            .await
            .map_err(|_| CatalogError::NativeCatalogIncomplete {
                table: owner.table.clone(),
                detail: format!("SQLite columns for index {name} are unavailable"),
            })?;
        let mut columns = Vec::new();
        let mut expression = false;
        for index_row in index_rows {
            match index_row.try_get::<Option<String>, _>("name") {
                Ok(Some(column)) => columns.push(column),
                Ok(None) => expression = true,
                Err(_) => expression = true,
            }
        }
        let definition = objects
            .iter()
            .find(|object| object.kind == "index" && object.name == name)
            .and_then(|object| object.definition.as_deref());
        indexes.push(NativeIndex {
            name,
            columns,
            unique: row.try_get::<i64, _>("unique").unwrap_or(0) != 0,
            partial_predicate: definition.and_then(index_predicate),
            expression,
        });
    }
    Ok(indexes)
}

fn native_column_type(declared_type: &str) -> NativeColumnType {
    let upper = declared_type.to_ascii_uppercase();
    if upper == "BOOLEAN" || upper == "BOOL" {
        NativeColumnType::Boolean
    } else if upper.contains("INT") {
        NativeColumnType::BigInt
    } else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        NativeColumnType::DoublePrecision
    } else if upper.contains("BLOB") {
        NativeColumnType::Bytea
    } else {
        NativeColumnType::Text
    }
}

fn native_resource_reference(column: &str) -> Option<NativeResourceReferenceKind> {
    let column = column.to_ascii_lowercase();
    if column == "raw_path" || column.ends_with("_raw_path") {
        Some(NativeResourceReferenceKind::Raw)
    } else if column == "archive_file_path" {
        Some(NativeResourceReferenceKind::ArchiveFile)
    } else if column == "file_path"
        || column == "staged_file_path"
        || column == "replacement_staged_path"
    {
        Some(NativeResourceReferenceKind::StagedFile)
    } else {
        None
    }
}

fn index_predicate(definition: &str) -> Option<String> {
    let upper = definition.to_ascii_uppercase();
    upper.find(" WHERE ").map(|position| {
        definition[position + " WHERE ".len()..]
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_owned()
    })
}

fn extract_check_constraints(definition: Option<&str>) -> Vec<String> {
    let Some(definition) = definition else {
        return Vec::new();
    };
    let bytes = definition.as_bytes();
    let mut checks = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[index];
                skip_quoted(definition, &mut index, quote);
                continue;
            }
            b'[' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b']' {
                    index += 1;
                }
                index = (index + 1).min(bytes.len());
                continue;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                continue;
            }
            _ => {}
        }
        if is_sql_identifier_byte(bytes[index]) {
            let start = index;
            while index < bytes.len() && is_sql_identifier_byte(bytes[index]) {
                index += 1;
            }
            if bytes[start..index].eq_ignore_ascii_case(b"CHECK") {
                let mut open = index;
                skip_sql_trivia(bytes, &mut open);
                if bytes.get(open) == Some(&b'(')
                    && let Some(close) = matching_parenthesis(definition, open)
                {
                    checks.push(definition[open + 1..close].trim().to_owned());
                    index = close + 1;
                }
            }
            continue;
        }
        index += 1;
    }
    checks
}

fn is_sql_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || !byte.is_ascii()
}

fn skip_sql_trivia(bytes: &[u8], index: &mut usize) {
    while *index < bytes.len() {
        if bytes[*index].is_ascii_whitespace() {
            *index += 1;
        } else if bytes.get(*index..*index + 2) == Some(b"--") {
            *index += 2;
            while *index < bytes.len() && bytes[*index] != b'\n' {
                *index += 1;
            }
        } else if bytes.get(*index..*index + 2) == Some(b"/*") {
            *index += 2;
            while *index + 1 < bytes.len() && &bytes[*index..*index + 2] != b"*/" {
                *index += 1;
            }
            *index = (*index + 2).min(bytes.len());
        } else {
            break;
        }
    }
}

fn matching_parenthesis(value: &str, open: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut depth = 0;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[index];
                skip_quoted(value, &mut index, quote);
                continue;
            }
            b'[' => {
                index += 1;
                while index < bytes.len() && bytes[index] != b']' {
                    index += 1;
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn normalize_native_default(value: &str) -> Option<String> {
    let mut value = value.trim();
    loop {
        let bytes = value.as_bytes();
        if bytes.first() == Some(&b'(') && bytes.last() == Some(&b')') {
            value = value[1..value.len() - 1].trim();
        } else {
            break;
        }
    }
    let lower = value.to_ascii_lowercase();
    if lower == "datetime('now')" || lower == "current_timestamp" {
        return Some("CURRENT_TIMESTAMP".into());
    }
    if let Some(translated) = normalize_known_strftime_default(value) {
        return Some(translated.into());
    }
    if sqlite_expression_detail(value).is_some() {
        return None;
    }
    if lower == "true" || lower == "false" || lower == "null" {
        return Some(value.to_owned());
    }
    if value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok() {
        return Some(value.to_owned());
    }
    if value.starts_with('\'') && value.ends_with('\'') {
        return Some(value.to_owned());
    }
    None
}

fn normalize_known_strftime_default(value: &str) -> Option<&'static str> {
    // Whitespace inside the format string and the case of conversion tokens are significant.
    if !value.get(..9)?.eq_ignore_ascii_case("strftime(") {
        return None;
    }
    let arguments = value.get(9..)?.trim_start().strip_prefix('\'')?;
    let end = arguments.find('\'')?;
    let format = &arguments[..end];
    let tail = arguments[end + 1..].trim_start().strip_prefix(',')?;
    let tail = tail.trim_start().strip_prefix("'now'")?.trim_start();
    if tail != ")" {
        return None;
    }
    match format {
        "%Y-%m-%dT%H:%M:%fZ" => Some(
            "to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')",
        ),
        "%Y-%m-%d %H:%M:%f" => {
            Some("to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.MS')")
        }
        _ => None,
    }
}

fn native_capture_metadata() -> NativeCaptureMetadata {
    NativeCaptureMetadata {
        log_table: "migration_change_log".into(),
        state_table: "migration_change_log_state".into(),
        table_name_column: "table_name".into(),
        stable_key_column: "stable_key".into(),
        operation_column: "kind".into(),
        source_identity_column: "source_identity".into(),
        source_version_column: "source_version".into(),
        generation_column: "capture_generation".into(),
        delete: NativeCaptureDeleteSemantics {
            operation_column: "kind".into(),
            delete_value: "delete".into(),
        },
    }
}

fn normalize_native_check(table: &str, check: &str) -> Result<String, CatalogError> {
    // These exact checks are generated by the two six-character ID schemas. SQLite GLOB's
    // negated class rejects any character outside the literal alphabet; PostgreSQL's !~ with
    // the equivalent negated class preserves that rule, including NULL check semantics.
    let translated = match (table, check.trim()) {
        (
            "hourly_invoke_prefixes",
            "length(prefix)=6 AND prefix NOT GLOB '*[^ABCDEFGHJKMNPQRSTUVWXYZ23456789]*'",
        ) => Some("length(prefix) = 6 AND prefix !~ '[^ABCDEFGHJKMNPQRSTUVWXYZ23456789]'"),
        (
            "prompt_cache_conversations",
            "conversation_id NOT GLOB '*[^ABCDEFGHJKMNPQRSTUVWXYZ23456789]*'",
        ) => Some("conversation_id !~ '[^ABCDEFGHJKMNPQRSTUVWXYZ23456789]'"),
        _ => None,
    };
    if let Some(translated) = translated {
        return Ok(translated.to_owned());
    }
    ensure_native_expression_supported(table, "CHECK", check)?;
    Ok(check.to_owned())
}

fn normalize_native_index_predicate(
    table: &str,
    index: &str,
    predicate: &str,
) -> Result<String, CatalogError> {
    // The source's two RFC3339 indexes test for the literal ASCII 'T' in a TEXT column.
    // strpos has the same positive/zero/NULL behavior for this predicate as SQLite instr.
    let canonical = predicate.split_whitespace().collect::<Vec<_>>().join(" ");
    let translated = match (table, index, canonical.as_str()) {
        (
            "codex_invocations",
            "idx_codex_invocations_long_term_projection_rfc3339_occurred_at",
            "instr(occurred_at, 'T') > 0 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
        ) => Some(
            "strpos(occurred_at, 'T') > 0 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
        ),
        (
            "codex_invocations",
            "idx_codex_invocations_long_term_projection_rfc3339_duration",
            "instr(occurred_at, 'T') > 0 AND t_total_ms IS NOT NULL AND t_total_ms > 0 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
        ) => Some(
            "strpos(occurred_at, 'T') > 0 AND t_total_ms IS NOT NULL AND t_total_ms > 0 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
        ),
        _ => None,
    };
    if let Some(translated) = translated {
        return Ok(translated.to_owned());
    }
    ensure_native_expression_supported(table, "partial index predicate", predicate)?;
    Ok(predicate.to_owned())
}

fn ensure_native_expression_supported(
    table: &str,
    context: &str,
    expression: &str,
) -> Result<(), CatalogError> {
    if let Some(expression_name) = sqlite_expression_detail(expression) {
        return Err(CatalogError::NativeCatalogIncomplete {
            table: table.into(),
            detail: format!("unsupported SQLite-only expression {expression_name} in {context}"),
        });
    }
    Ok(())
}

fn sqlite_expression_detail(value: &str) -> Option<&'static str> {
    const SQLITE_ONLY_EXPRESSIONS: &[(&str, &str)] = &[
        ("DATETIME(", "datetime()"),
        ("STRFTIME(", "strftime()"),
        ("JSON_VALID(", "json_valid()"),
        ("JSON_EXTRACT(", "json_extract()"),
        ("INSTR(", "instr()"),
        ("JULIANDAY(", "julianday()"),
        ("INSERT OR REPLACE", "INSERT OR REPLACE"),
    ];
    let upper = value.to_ascii_uppercase();
    SQLITE_ONLY_EXPRESSIONS
        .iter()
        .find_map(|(marker, name)| upper.contains(marker).then_some(*name))
        .or_else(|| {
            sql_tokens(value)
                .iter()
                .any(|token| token == "GLOB")
                .then_some("GLOB")
        })
}

fn normalize_fk_action(action: &str) -> &'static str {
    match action.to_ascii_uppercase().as_str() {
        "CASCADE" => "CASCADE",
        "SET NULL" => "SET NULL",
        "SET DEFAULT" => "SET DEFAULT",
        "RESTRICT" => "RESTRICT",
        _ => "NO ACTION",
    }
}

fn postgres_fk_action_code(action: &str) -> &'static str {
    match action {
        "CASCADE" => "c",
        "SET NULL" => "n",
        "SET DEFAULT" => "d",
        "RESTRICT" => "r",
        _ => "a",
    }
}

fn native_text_array(values: &[String]) -> String {
    format!(
        "ARRAY[{}]::text[]",
        values
            .iter()
            .map(|value| quote_native_literal(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn quote_native_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_native_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn closure_contains_table(tables: &[NativeTableSpec], table: &str) -> bool {
    tables.iter().any(|spec| spec.table_name() == table)
}

fn trigger_write_tables(definition: &str) -> Vec<String> {
    let body = definition
        .find("BEGIN")
        .map_or(definition, |begin| &definition[begin + "BEGIN".len()..]);
    let tokens = sql_tokens(body);
    let mut targets = BTreeSet::new();
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            "INSERT" | "REPLACE" => {
                if let Some(into) = tokens[index + 1..]
                    .iter()
                    .take(4)
                    .position(|token| token == "INTO")
                    && let Some(target) = tokens.get(index + 2 + into)
                {
                    targets.insert(target.to_ascii_lowercase());
                }
            }
            "UPDATE" => {
                if tokens.get(index + 1).map(String::as_str) != Some("SET")
                    && let Some(target) = tokens.get(index + 1)
                {
                    targets.insert(target.to_ascii_lowercase());
                }
            }
            "DELETE" => {
                if let Some(from) = tokens[index + 1..]
                    .iter()
                    .take(2)
                    .position(|token| token == "FROM")
                    && let Some(target) = tokens.get(index + 2 + from)
                {
                    targets.insert(target.to_ascii_lowercase());
                }
            }
            _ => {}
        }
        index += 1;
    }
    targets.into_iter().collect()
}

fn sql_tokens(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => skip_quoted(sql, &mut index, b'\''),
            b'"' | b'`' => {
                let quote = bytes[index];
                index += 1;
                let start = index;
                while index < bytes.len() {
                    if bytes[index] == quote {
                        if bytes.get(index + 1) == Some(&quote) {
                            index += 2;
                            continue;
                        }
                        break;
                    }
                    index += 1;
                }
                if start < index {
                    tokens.push(sql[start..index].to_string());
                }
                if index < bytes.len() {
                    index += 1;
                }
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric()
                        || bytes[index] == b'_'
                        || bytes[index] == b'$')
                {
                    index += 1;
                }
                tokens.push(sql[start..index].to_ascii_uppercase());
            }
            _ => index += 1,
        }
    }
    tokens
}

fn skip_quoted(sql: &str, index: &mut usize, quote: u8) {
    let bytes = sql.as_bytes();
    *index += 1;
    while *index < bytes.len() {
        if bytes[*index] == quote {
            if bytes.get(*index + 1) == Some(&quote) {
                *index += 2;
                continue;
            }
            *index += 1;
            break;
        }
        *index += 1;
    }
}

fn traffic_group(table: &str) -> Result<&'static str, CatalogError> {
    match table {
        "codex_invocations"
        | "pool_upstream_request_attempts"
        | "forward_proxy_attempts"
        | "forward_proxy_metadata_history"
        | "proxy_raw_payload_blobs"
        | "proxy_raw_payload_blob_links"
        | "proxy_raw_payload_blob_link_migrations"
        | "codex_invocation_raw_codec_migrations"
        | "codex_quota_snapshots"
        | "invocation_in_progress_live"
        | "prompt_cache_working_set_live" => Ok("traffic_fact"),
        "archive_batches"
        | "retention_prepared_archives"
        | "retention_recovery_cursors"
        | "retention_raw_reconciliation"
        | "hourly_rollup_archive_replay"
        | "hourly_rollup_materialized_buckets"
        | "archive_batch_upstream_activity"
        | "pool_upstream_node_health_archive"
        | "pool_upstream_node_health_hourly_archive" => Ok("archive_publication"),
        "summary_source_change_journal"
        | "summary_source_change_cursor"
        | "summary_source_change_compaction_proof"
        | "summary_coverage_obligation"
        | "summary_coverage_revision"
        | "summary_account_coverage_revision"
        | "summary_all_time_coverage_checkpoint"
        | "summary_live_tail_reconciliation_checkpoint"
        | "summary_archive_snapshot"
        | "summary_archive_snapshot_v2_proof"
        | "summary_archive_snapshot_backfill_checkpoint"
        | "summary_archive_snapshot_backfill_outcome"
        | "hourly_rollup_archive_progress"
        | "hourly_rollup_archive_repair_progress"
        | "hourly_rollup_archive_repair_deferred"
        | "hourly_rollup_live_progress"
        | "hourly_rollup_repair_seen_invocation_ids"
        | "hourly_rollup_repair_cleared_buckets"
        | "account_activity_v2_bucket_repair_watermarks" => Ok("traffic_recovery_obligation"),
        _ => Err(CatalogError::UnknownOwnership(table.into())),
    }
}

fn traffic_table(name: &str) -> bool {
    matches!(
        name,
        "codex_invocations"
            | "invocation_in_progress_live"
            | "prompt_cache_working_set_live"
            | "pool_upstream_request_attempts"
            | "forward_proxy_attempts"
            | "forward_proxy_metadata_history"
            | "proxy_raw_payload_blobs"
            | "proxy_raw_payload_blob_links"
            | "proxy_raw_payload_blob_link_migrations"
            | "codex_invocation_raw_codec_migrations"
            | "codex_quota_snapshots"
            | "archive_batches"
            | "retention_prepared_archives"
            | "retention_recovery_cursors"
            | "retention_raw_reconciliation"
            | "archive_batch_upstream_activity"
            | "hourly_rollup_materialized_buckets"
            | "account_activity_v2_bucket_repair_watermarks"
            | "hourly_rollup_archive_replay"
            | "pool_upstream_node_health_archive"
            | "pool_upstream_node_health_hourly_archive"
            | "summary_source_change_journal"
            | "summary_source_change_cursor"
            | "summary_source_change_compaction_proof"
            | "summary_coverage_obligation"
            | "summary_coverage_revision"
            | "summary_account_coverage_revision"
            | "summary_all_time_coverage_checkpoint"
            | "summary_live_tail_reconciliation_checkpoint"
            | "summary_archive_snapshot"
            | "summary_archive_snapshot_v2_proof"
            | "summary_archive_snapshot_backfill_checkpoint"
            | "summary_archive_snapshot_backfill_outcome"
            | "hourly_rollup_archive_progress"
            | "hourly_rollup_archive_repair_progress"
            | "hourly_rollup_archive_repair_deferred"
            | "hourly_rollup_live_progress"
            | "hourly_rollup_repair_seen_invocation_ids"
            | "hourly_rollup_repair_cleared_buckets"
    )
}

fn analytics_table(name: &str) -> bool {
    matches!(
        name,
        "analytics_projection_checkpoints"
            | "invocation_rollup_daily"
            | "invocation_rollup_hourly"
            | "invocation_failure_rollup_hourly"
            | "proxy_perf_stage_hourly"
            | "prompt_cache_rollup_hourly"
            | "prompt_cache_upstream_account_hourly"
            | "parallel_work_minute_key_rollup"
            | "parallel_work_upstream_account_minute_key_rollup"
            | "parallel_work_hourly_rollup"
            | "parallel_work_upstream_account_hourly_rollup"
            | "parallel_work_hourly_coverage"
            | "parallel_work_rollup_coverage_state"
            | "parallel_work_rollup_maintenance_state"
            | "timeseries_minute_projection_records"
            | "timeseries_minute_projection_v2"
            | "timeseries_minute_projection_v2_state"
            | "timeseries_minute_projection_v2_recovery"
            | "upstream_account_usage_hourly"
            | "upstream_account_usage_breakdown_hourly"
            | "upstream_account_stats_hourly"
            | "upstream_account_stats_minute"
            | "upstream_host_network_minute"
            | "upstream_socket_network_minute"
            | "upstream_sticky_key_hourly"
            | "forward_proxy_attempt_hourly"
            | "forward_proxy_weight_hourly"
            | "dashboard_activity_excluded_invocation_ids"
            | "hourly_invoke_prefixes"
            | "prompt_cache_conversations"
            | "prompt_cache_conversation_stats_refresh_queue"
            | "prompt_cache_conversation_stats_generation_clock"
            | "prompt_cache_conversation_stats_freshness"
            | "prompt_cache_conversation_stats_refresh_staging"
            | "prompt_cache_conversation_migration_progress"
            | "prompt_cache_conversation_materialization_runs"
            | "prompt_cache_conversation_orphan_cleanup_state"
            | "invocation_timeline_snapshot_rows"
            | "long_term_projection_archive_compatibility"
            | "long_term_projection_bucket_state"
            | "long_term_projection_daily_backup_claims"
            | "long_term_projection_daily_backups"
            | "long_term_projection_date_publications"
            | "long_term_projection_dirty_buckets"
            | "long_term_projection_interval_state"
            | "long_term_projection_interval_suppressions"
            | "long_term_projection_intervals"
            | "long_term_projection_rebuild_members"
            | "long_term_projection_state"
            | "long_term_stats_repair_queue"
            | "long_term_stats_state"
            | "long_term_usage_daily"
            | "long_term_usage_hourly"
            | "retention_backlog_hourly_observations"
            | "summary_all_time_projection_checkpoint"
            | "summary_all_time_projection_account_checkpoint"
            | "summary_all_time_projection_usage_checkpoint"
            | "summary_compacted_source_recovery_checkpoint"
            | "summary_compacted_source_recovery_aggregate"
            | "summary_compacted_source_recovery_usage"
            | "summary_compacted_source_recovery_record"
    )
}

fn account_table(name: &str) -> bool {
    matches!(
        name,
        "external_api_keys"
            | "managed_model_catalog_migrations"
            | "managed_model_suppressions"
            | "managed_models"
            | "models_dev_sync_discovered_models"
            | "models_dev_sync_metadata"
            | "models_dev_sync_model_selections"
            | "models_dev_sync_provider_selections"
            | "models_dev_sync_quote_provider_choices"
            | "oauth_bridge_settings"
            | "pool_oauth_login_sessions"
            | "pool_oauth_mailbox_sessions"
            | "pool_routing_settings"
            | "pool_sticky_model_route_generations"
            | "pool_sticky_model_routes"
            | "pool_sticky_route_generations"
            | "pool_sticky_routes"
            | "pool_tags"
            | "pool_upstream_account_egress_throttle"
            | "pool_upstream_account_events"
            | "pool_upstream_account_group_notes"
            | "pool_upstream_account_limit_samples"
            | "pool_upstream_account_model_catalogs"
            | "pool_upstream_account_model_routes"
            | "pool_upstream_account_selected_touch_outbox"
            | "pool_upstream_account_long_term_projection_outbox"
            | "pool_upstream_account_tags"
            | "pool_upstream_accounts"
            | "prompt_cache_conversation_bindings"
            | "prompt_cache_conversation_operation_events"
            | "prompt_cache_encrypted_session_owners"
            | "proxy_model_settings"
            | "pricing_settings_meta"
            | "pricing_settings_models"
            | "pricing_model_quotes"
            | "forward_proxy_settings"
            | "forward_proxy_runtime"
    )
}

fn task_table(name: &str) -> bool {
    matches!(
        name,
        "schema_refresh_migrations"
            | "storage_migration_runtime"
            | "storage_migration_authority"
            | "storage_migration_engine_checkpoints"
            | "storage_migration_engine_outbox"
            | "storage_migration_engine_reservations"
            | "storage_migration_engine_status"
            | "startup_backfill_progress"
            | "system_task_runs"
            | "maintenance_metadata"
            | "maintenance_owner_retry_state"
            | "managed_task_progress"
            | "managed_task_runs"
            | "managed_task_work_runs"
            | "managed_tasks"
            | "task_timeline_coverage"
            | "task_timeline_segments"
    )
}

fn performance_table(name: &str) -> bool {
    matches!(
        name,
        "system_raw_payload_inventory_paths" | "system_raw_payload_metrics"
    )
}
