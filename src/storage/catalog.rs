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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TableOwner {
    pub(crate) table: String,
    pub(crate) module: ModuleId,
    pub(crate) transaction_group: &'static str,
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
        let catalog = Self {
            owners,
            objects,
            foreign_keys,
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

    pub(crate) fn validate_transaction<'a>(
        &self,
        tables: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), CatalogError> {
        let mut module = None;
        for table in tables {
            let owner = self
                .owners
                .iter()
                .find(|owner| owner.table == table)
                .map(|owner| owner.module)
                .ok_or_else(|| CatalogError::UnknownOwnership(table.into()))?;
            if let Some(first) = module {
                if first != owner {
                    return Err(CatalogError::CrossModuleTransaction {
                        first,
                        other: owner,
                        table: table.into(),
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
    let (module, transaction_group) = if traffic_table(&table) {
        (ModuleId::TrafficArchive, traffic_group(&table))
    } else if analytics_table(&table) {
        (ModuleId::Analytics, "analytics_projection")
    } else if account_table(&table) {
        (ModuleId::AccountRouting, "account_control")
    } else if task_table(&table) {
        (ModuleId::TaskOperations, "task_operations")
    } else if performance_table(&table) {
        (ModuleId::Performance, "performance")
    } else {
        return Err(CatalogError::UnknownOwnership(table));
    };
    Ok(TableOwner {
        table,
        module,
        transaction_group,
    })
}

fn traffic_group(table: &str) -> &'static str {
    match table {
        "codex_invocations"
        | "pool_upstream_request_attempts"
        | "forward_proxy_attempts"
        | "proxy_raw_payload_blobs"
        | "proxy_raw_payload_blob_links"
        | "invocation_in_progress_live"
        | "prompt_cache_working_set_live" => "traffic_fact",
        "archive_batches"
        | "retention_prepared_archives"
        | "retention_recovery_cursors"
        | "retention_raw_reconciliation"
        | "hourly_rollup_archive_replay"
        | "hourly_rollup_materialized_buckets"
        | "archive_batch_upstream_activity"
        | "pool_upstream_node_health_hourly_archive" => "archive_publication",
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
        | "account_activity_v2_bucket_repair_watermarks" => "traffic_recovery_obligation",
        _ => "traffic_archive_fact",
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
