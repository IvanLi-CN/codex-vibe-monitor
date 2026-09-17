use super::*;

#[path = "ensure_schema_phases/account_stats_hourly.rs"]
mod account_stats_hourly;
#[path = "ensure_schema_phases/account_stats_minute.rs"]
mod account_stats_minute;
#[path = "ensure_schema_phases/archive.rs"]
mod archive;
#[path = "ensure_schema_phases/defaults_and_maintenance.rs"]
mod defaults_and_maintenance;
#[path = "ensure_schema_phases/final_reconciliation.rs"]
mod final_reconciliation;
#[path = "ensure_schema_phases/foundation.rs"]
mod foundation;
#[path = "ensure_schema_phases/invocation_indexes.rs"]
mod invocation_indexes;
#[path = "ensure_schema_phases/live_projection.rs"]
mod live_projection;
#[path = "ensure_schema_phases/prompt_cache_and_attempts.rs"]
mod prompt_cache_and_attempts;
#[path = "ensure_schema_phases/proxy_settings.rs"]
mod proxy_settings;
#[path = "ensure_schema_phases/rollups.rs"]
mod rollups;
#[path = "ensure_schema_phases/timeseries.rs"]
mod timeseries;

#[derive(Default)]
struct SchemaAccountStatsState {
    upstream_account_usage_hourly_needs_status_backfill: bool,
    upstream_account_stats_hourly_count: i64,
    upstream_account_stats_minute_count: i64,
    added_upstream_account_stats_columns: bool,
}

pub(crate) async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let schema_lock = ensure_schema_lock(pool);
    let _schema_guard = schema_lock.lock_owned().await;
    foundation::ensure_schema_foundation(pool).await?;
    invocation_indexes::ensure_schema_invocation_indexes(pool).await?;
    live_projection::ensure_schema_live_projection(pool).await?;
    archive::ensure_schema_archive(pool).await?;
    let (has_existing_rollup_rows, added_rollup_columns) =
        rollups::ensure_schema_rollups(pool).await?;
    timeseries::ensure_schema_timeseries(pool).await?;
    let mut state = SchemaAccountStatsState::default();
    account_stats_hourly::ensure_schema_account_stats_hourly(pool, &mut state).await?;
    account_stats_minute::ensure_schema_account_stats_minute(pool, &mut state).await?;
    final_reconciliation::ensure_schema_final_reconciliation(
        pool,
        has_existing_rollup_rows,
        added_rollup_columns,
        &state,
    )
    .await?;
    proxy_settings::ensure_schema_proxy_settings(pool).await?;
    prompt_cache_and_attempts::ensure_schema_prompt_cache_and_attempts(pool).await?;
    defaults_and_maintenance::ensure_schema_defaults_and_maintenance(pool).await?;
    Ok(())
}
