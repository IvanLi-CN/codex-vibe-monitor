use super::*;

pub(crate) async fn ensure_upstream_accounts_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_pool_upstream_accounts_table(pool).await?;
    ensure_pool_account_model_catalog(pool).await?;
    ensure_pool_account_columns_core(pool).await?;
    ensure_pool_account_columns_capabilities(pool).await?;
    ensure_pool_account_columns_policies(pool).await?;
    ensure_pool_account_columns_timeout(pool).await?;
    ensure_pool_account_columns_metadata(pool).await?;
    ensure_pool_account_indexes_and_backfills(pool).await?;
    ensure_external_api_keys(pool).await?;
    ensure_pool_account_events_table(pool).await?;
    ensure_pool_account_events_columns_and_indexes(pool).await?;
    ensure_pool_account_model_routes_table(pool).await?;
    ensure_pool_account_model_route_columns(pool).await?;
    ensure_pool_account_model_route_indexes(pool).await?;
    ensure_pool_oauth_session_tables(pool).await?;
    ensure_pool_oauth_session_columns(pool).await?;
    ensure_pool_oauth_session_pending_columns(pool).await?;
    ensure_pool_oauth_session_indexes(pool).await?;
    ensure_pool_tags(pool).await?;
    ensure_pool_oauth_mailbox(pool).await?;
    ensure_pool_group_notes_table(pool).await?;
    ensure_pool_group_notes_legacy_columns(pool).await?;
    ensure_pool_group_notes_policy_columns(pool).await?;
    ensure_pool_group_notes_extended_policy(pool).await?;
    ensure_pool_limit_samples(pool).await?;
    ensure_pool_sticky_routes(pool).await?;
    ensure_pool_sticky_route_generations(pool).await?;
    ensure_pool_routing_settings_table(pool).await?;
    let routing_models_mode_was_missing = ensure_pool_routing_settings_columns(pool).await?;
    finish_pool_routing_settings(pool, routing_models_mode_was_missing).await?;
    Ok(())
}
