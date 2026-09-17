use super::*;

pub(super) async fn ensure_summary_projection_checkpoint(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_projection_checkpoint (
            scope TEXT PRIMARY KEY,
            live_high_watermark_id INTEGER NOT NULL,
            rollup_live_cursor INTEGER NOT NULL,
            account_rollup_live_cursor INTEGER,
            manifest_high_watermark_id INTEGER,
            durable_terminal_sequence_watermark INTEGER NOT NULL,
            global_manifest_next_id INTEGER NOT NULL DEFAULT 0,
            account_manifest_next_id INTEGER NOT NULL DEFAULT 0,
            global_manifest_complete INTEGER NOT NULL DEFAULT 0,
            account_manifest_complete INTEGER NOT NULL DEFAULT 0,
            global_rollup_next_rowid INTEGER NOT NULL DEFAULT 0,
            account_rollup_next_rowid INTEGER NOT NULL DEFAULT 0,
            usage_rollup_next_rowid INTEGER NOT NULL DEFAULT 0,
            global_rollup_complete INTEGER NOT NULL DEFAULT 0,
            account_rollup_complete INTEGER NOT NULL DEFAULT 0,
            usage_rollup_complete INTEGER NOT NULL DEFAULT 0,
            account_unavailable INTEGER NOT NULL DEFAULT 0,
            global_usage_unavailable INTEGER NOT NULL DEFAULT 0,
            account_usage_unavailable INTEGER NOT NULL DEFAULT 0,
            global_total_count INTEGER NOT NULL DEFAULT 0,
            global_success_count INTEGER NOT NULL DEFAULT 0,
            global_failure_count INTEGER NOT NULL DEFAULT 0,
            global_total_tokens INTEGER NOT NULL DEFAULT 0,
            global_non_success_tokens INTEGER NOT NULL DEFAULT 0,
            global_total_cost REAL NOT NULL DEFAULT 0,
            global_non_success_cost REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_projection_checkpoint table existence")?;
    ensure_column_with_definition(
        pool,
        "summary_all_time_projection_checkpoint",
        "coverage_revision",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_all_time_projection_checkpoint",
        "account_coverage_revision",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    Ok(())
}

pub(super) async fn ensure_account_projection_checkpoint(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_projection_account_checkpoint (
            scope TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            non_success_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
            non_success_cost REAL NOT NULL DEFAULT 0,
            PRIMARY KEY (scope, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_projection_account_checkpoint table existence")?;

    for (column, definition) in [
        ("usage_rollup_next_rowid", "INTEGER NOT NULL DEFAULT 0"),
        ("usage_rollup_complete", "INTEGER NOT NULL DEFAULT 0"),
        ("global_usage_unavailable", "INTEGER NOT NULL DEFAULT 0"),
        ("account_usage_unavailable", "INTEGER NOT NULL DEFAULT 0"),
        ("global_non_success_tokens", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_column_with_definition(
            pool,
            "summary_all_time_projection_checkpoint",
            column,
            definition,
        )
        .await?;
    }
    ensure_column_with_definition(
        pool,
        "summary_all_time_projection_account_checkpoint",
        "non_success_tokens",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    Ok(())
}

pub(super) async fn ensure_usage_projection_checkpoint(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_all_time_projection_usage_checkpoint (
            scope TEXT NOT NULL,
            aggregate_scope TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL DEFAULT 0,
            normalized_model TEXT NOT NULL,
            normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
            cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cost_input REAL NOT NULL DEFAULT 0,
            cost_cache_write REAL NOT NULL DEFAULT 0,
            cost_cache_read REAL NOT NULL DEFAULT 0,
            cost_output REAL NOT NULL DEFAULT 0,
            cost_reasoning REAL NOT NULL DEFAULT 0,
            cost_unknown REAL NOT NULL DEFAULT 0,
            has_cost INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (
                scope,
                aggregate_scope,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_projection_usage_checkpoint table existence")?;
    Ok(())
}
