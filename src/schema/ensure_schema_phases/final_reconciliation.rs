async fn ensure_schema_final_reconciliation(
    pool: &Pool<Sqlite>,
    has_existing_rollup_rows: bool,
    added_rollup_columns: bool,
    state: &SchemaAccountStatsState,
) -> Result<()> {
    if has_existing_rollup_rows {
        let reconciliation_complete = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE((SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1), 0)",
        )
        .bind(INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET)
        .fetch_one(pool)
        .await?
            != 0;
        if !reconciliation_complete || added_rollup_columns {
            let reconciliation = backfill_invocation_rollup_hourly_from_sources(pool).await?;
            if reconciliation.source_complete {
                sqlx::query(
                    r#"
                    INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
                    VALUES (?1, 1)
                    ON CONFLICT(dataset) DO UPDATE SET
                        cursor_id = excluded.cursor_id,
                        updated_at = datetime('now')
                    "#,
                )
                .bind(INVOCATION_ROLLUP_TOKEN_COMPONENT_RECONCILIATION_DATASET)
                .execute(pool)
                .await?;
            }
            info!(
                rebuilt_rows = reconciliation.applied_rollups,
                source_complete = reconciliation.source_complete,
                "backfilled invocation hourly rollups after adding aggregate columns"
            );
        }
    }

    if state.upstream_account_usage_hourly_needs_status_backfill {
        backfill_upstream_account_usage_hourly_status_counts(pool).await?;
    }
    if state.upstream_account_stats_hourly_count == 0
        || state.upstream_account_stats_minute_count == 0
        || state.added_upstream_account_stats_columns
    {
        rebuild_upstream_account_stats_rollups_from_sources(pool)
            .await
            .context("failed to rebuild upstream account stats rollups from sources")?;
    }

    Ok(())
}
