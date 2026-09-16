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
    ensure_schema_foundation(pool).await?;
    ensure_schema_invocation_indexes(pool).await?;
    ensure_schema_live_projection(pool).await?;
    ensure_schema_archive(pool).await?;
    let (has_existing_rollup_rows, added_rollup_columns) = ensure_schema_rollups(pool).await?;
    ensure_schema_timeseries(pool).await?;
    let mut state = SchemaAccountStatsState::default();
    ensure_schema_account_stats_hourly(pool, &mut state).await?;
    ensure_schema_account_stats_minute(pool, &mut state).await?;
    ensure_schema_final_reconciliation(
        pool,
        has_existing_rollup_rows,
        added_rollup_columns,
        &state,
    )
    .await?;
    ensure_schema_proxy_settings(pool).await?;
    ensure_schema_prompt_cache_and_attempts(pool).await?;
    ensure_schema_defaults_and_maintenance(pool).await?;
    Ok(())
}

include!("ensure_schema_phases/foundation.rs");
include!("ensure_schema_phases/invocation_indexes.rs");
include!("ensure_schema_phases/live_projection.rs");
include!("ensure_schema_phases/archive.rs");
include!("ensure_schema_phases/rollups.rs");
include!("ensure_schema_phases/timeseries.rs");
include!("ensure_schema_phases/account_stats_hourly.rs");
include!("ensure_schema_phases/account_stats_minute.rs");
include!("ensure_schema_phases/final_reconciliation.rs");
include!("ensure_schema_phases/proxy_settings.rs");
include!("ensure_schema_phases/prompt_cache_and_attempts.rs");
include!("ensure_schema_phases/defaults_and_maintenance.rs");
