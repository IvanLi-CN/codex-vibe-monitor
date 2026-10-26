use std::{collections::BTreeSet, fmt};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TriggerWriteRelation {
    pub(crate) trigger: String,
    pub(crate) source_table: String,
    pub(crate) target_table: String,
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
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleCatalog {
    owners: Vec<TableOwner>,
    objects: Vec<SchemaObject>,
    foreign_keys: Vec<ForeignKeyRelation>,
    trigger_writes: Vec<TriggerWriteRelation>,
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
                owners.push(classify_table(name.clone())?);
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
            for row in sqlx::query(&query).fetch_all(pool).await.map_err(|_| {
                CatalogError::UnknownOwnership(format!(
                    "<foreign keys for {} unavailable>",
                    owner.table
                ))
            })? {
                let referenced_table = row.try_get::<String, _>("table").map_err(|_| {
                    CatalogError::UnknownOwnership(format!(
                        "<invalid foreign key target for {}>",
                        owner.table
                    ))
                })?;
                foreign_keys.push(ForeignKeyRelation {
                    table: owner.table.clone(),
                    referenced_table,
                });
            }
        }
        let mut trigger_writes = Vec::new();
        for object in objects.iter().filter(|object| object.kind == "trigger") {
            let definition = object.definition.as_deref().ok_or_else(|| {
                CatalogError::UnknownOwnership(format!(
                    "<trigger definition for {} unavailable>",
                    object.name
                ))
            })?;
            for target_table in trigger_write_tables(definition) {
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
        let catalog = Self {
            owners,
            objects,
            foreign_keys,
            trigger_writes,
        };
        catalog.validate_schema_object_owners()?;
        catalog.validate_foreign_key_ownership()?;
        Ok(catalog)
    }

    pub(crate) fn owners(&self) -> &[TableOwner] {
        &self.owners
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
        "invocation_rollup_daily"
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
