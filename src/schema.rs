use super::*;

mod archive_retention;
mod compatibility;
mod invocation_live;
mod prompt_cache_working_set_triggers;
mod rollups_statistics;
mod runtime_system;
mod summary_coverage;

#[expect(
    unused_imports,
    reason = "Schema facade re-exports preserve crate-visible migration and query helper paths."
)]
pub(crate) use {
    archive_retention::{
        migrate_pool_upstream_node_health_hourly_archive_identity,
        pool_upstream_node_health_hourly_archive_create_sql,
    },
    compatibility::{
        ENSURE_SCHEMA_LOCKS, INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL,
        INVOCATION_UPSTREAM_ACCOUNT_ID_EXPR_SQL, PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS,
        SHANGHAI_NOW_SQL, ensure_schema_lock, ensure_schema_lock_key,
    },
    invocation_live::{
        ensure_reported_cache_write_tokens_column,
        invocation_in_progress_live_prompt_cache_key_expr,
        invocation_in_progress_live_refresh_set_clause,
        invocation_in_progress_live_refresh_sql_for_key, invocation_in_progress_live_upsert_sql,
        invocation_in_progress_live_upstream_account_id_expr,
        migrate_prompt_cache_conversation_bindings_contract, prompt_cache_binding_copy_expr,
        prompt_cache_conversation_bindings_create_sql,
        prompt_cache_conversation_bindings_existing_columns,
        prompt_cache_conversation_operation_events_create_sql,
        prompt_cache_working_set_live_refresh_sql_for_key,
        rebuild_invocation_in_progress_live_table, rebuild_invocation_in_progress_live_triggers,
        rebuild_prompt_cache_working_set_live_table,
    },
    rollups_statistics::{
        backfill_upstream_account_usage_hourly_status_counts,
        reopen_upstream_account_stats_rollup_archives,
    },
    runtime_system::{proxy_raw_blob_link_seed_completed, retire_openai_websocket_proxy},
};

pub(crate) async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let schema_lock = ensure_schema_lock(pool);
    let _schema_guard = schema_lock.lock_owned().await;

    // Existing live-update triggers may already reference this table after an interrupted older
    // startup. Restore it before any schema work can update invocations.
    sqlx::query(compatibility::TIMESERIES_MINUTE_PROJECTION_V2_RECOVERY_TABLE_SQL)
        .execute(pool)
        .await
        .context("failed to ensure timeseries_minute_projection_v2 recovery table existence")?;
    compatibility::ensure_schema_refresh_migrations_table(pool).await?;

    invocation_live::ensure_invocation_live_schema(pool).await?;
    archive_retention::ensure_archive_retention_schema(pool).await?;
    let rollup_backfill_state = rollups_statistics::ensure_rollup_statistics_schema(pool).await?;
    archive_retention::ensure_hourly_archive_schema(pool).await?;
    summary_coverage::ensure_summary_coverage_schema(pool).await?;
    rollups_statistics::finish_rollup_backfills(pool, rollup_backfill_state).await?;
    runtime_system::ensure_runtime_system_schema(pool).await?;
    summary_coverage::ensure_summary_coverage_revision_schema(pool).await?;
    crate::storage::migration_status::ensure_persistence_schema(pool).await?;

    Ok(())
}
