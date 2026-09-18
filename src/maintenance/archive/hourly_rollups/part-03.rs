pub(crate) async fn upsert_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    targets: &[&str],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    upsert_parallel_work_minute_key_rollups_tx(tx, rows).await?;
    let target_flags = InvocationHourlyRollupTargets::from_targets(targets);
    let deltas = collect_invocation_hourly_rollup_deltas(rows, &target_flags)?;
    persist_invocation_hourly_rollup_deltas(tx, target_flags, deltas).await
}

async fn persist_invocation_hourly_rollup_deltas(
    tx: &mut SqliteConnection,
    targets: InvocationHourlyRollupTargets,
    deltas: InvocationHourlyRollupDeltas,
) -> Result<()> {
    if targets.overall {
        persist_invocation_hourly_overall_rollups_tx(tx, deltas.overall).await?;
    }
    if targets.failures {
        persist_invocation_hourly_failures_tx(tx, deltas.failures).await?;
    }
    if targets.perf {
        persist_invocation_hourly_perf_tx(tx, deltas.perf).await?;
    }
    if targets.prompt_cache {
        persist_invocation_hourly_prompt_cache_tx(tx, deltas.prompt_cache).await?;
    }
    if targets.prompt_cache_upstream_accounts {
        persist_invocation_hourly_prompt_cache_accounts_tx(
            tx,
            deltas.prompt_cache_upstream_accounts,
        )
        .await?;
    }
    if targets.upstream_account_usage {
        persist_invocation_hourly_account_usage_tx(tx, deltas.upstream_account_usage).await?;
    }
    if targets.upstream_account_usage_breakdown {
        persist_invocation_hourly_breakdown_tx(tx, deltas.upstream_account_usage_breakdown).await?;
    }
    if targets.upstream_account_stats_hourly {
        persist_invocation_hourly_account_stats_tx(tx, deltas.upstream_account_stats_hourly)
            .await?;
    }
    if targets.upstream_account_activity_v2 {
        persist_invocation_hourly_activity_tx(tx, deltas.upstream_account_activity_v2).await?;
    }
    if targets.upstream_account_stats_minute {
        persist_invocation_hourly_account_minute_stats_tx(tx, deltas.upstream_account_stats_minute)
            .await?;
    }
    if targets.sticky_keys {
        persist_invocation_hourly_sticky_keys_tx(tx, deltas.sticky_keys).await?;
    }
    Ok(())
}
