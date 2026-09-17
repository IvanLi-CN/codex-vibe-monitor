use super::*;

pub(super) async fn ensure_schema_account_stats_hourly(
    pool: &Pool<Sqlite>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    ensure_usage_hourly_schema(pool, state).await?;
    ensure_account_stats_hourly_schema(pool, state).await?;
    ensure_account_stats_minute_table(pool).await?;
    Ok(())
}

async fn ensure_usage_hourly_schema(
    pool: &Pool<Sqlite>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    ensure_usage_hourly_columns(pool, state).await?;
    ensure_usage_hourly_rollup_tables(pool).await?;
    Ok(())
}

async fn ensure_usage_hourly_columns(
    pool: &Pool<Sqlite>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    let upstream_account_usage_hourly_columns =
        load_sqlite_table_columns(pool, "upstream_account_usage_hourly").await?;
    state.upstream_account_usage_hourly_needs_status_backfill = false;
    for (column, ty) in [
        ("success_count", "INTEGER NOT NULL DEFAULT 0"),
        ("failure_count", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
    ] {
        if !upstream_account_usage_hourly_columns.contains(column) {
            state.upstream_account_usage_hourly_needs_status_backfill = true;
            let sql = format!("ALTER TABLE upstream_account_usage_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&sql).execute(pool).await.with_context(|| {
                format!("failed to add upstream_account_usage_hourly column {column}")
            })?;
        }
    }
    Ok(())
}

async fn ensure_usage_hourly_rollup_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_usage_hourly_account_bucket
        ON upstream_account_usage_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_usage_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_usage_breakdown_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            normalized_model TEXT NOT NULL,
            normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
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
            performance_total_tokens INTEGER NOT NULL DEFAULT 0,
            performance_stream_output_tokens INTEGER NOT NULL DEFAULT 0,
            performance_stream_duration_ms REAL NOT NULL DEFAULT 0,
            performance_response_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_response_sum_ms REAL NOT NULL DEFAULT 0,
            performance_first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            performance_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_first_token_sum_ms REAL NOT NULL DEFAULT 0,
            performance_usage_duration_sample_count INTEGER NOT NULL DEFAULT 0,
            performance_usage_duration_sum_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (
                bucket_start_epoch,
                source,
                upstream_account_key,
                normalized_model,
                normalized_reasoning_effort
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_usage_breakdown_hourly table existence")?;

    for (column, definition) in [
        (
            "performance_first_token_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("performance_first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
    ] {
        ensure_column_with_definition(
            pool,
            "upstream_account_usage_breakdown_hourly",
            column,
            definition,
        )
        .await?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_usage_breakdown_hourly_account_bucket
        ON upstream_account_usage_breakdown_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_usage_breakdown_hourly_account_bucket")?;
    Ok(())
}

async fn ensure_account_stats_hourly_schema(
    pool: &Pool<Sqlite>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    ensure_account_stats_hourly_table(pool).await?;
    ensure_account_stats_hourly_columns(pool, state).await?;
    ensure_account_stats_hourly_indexes(pool).await?;
    Ok(())
}

async fn ensure_account_stats_hourly_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_stats_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            in_flight_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
            non_success_cost REAL NOT NULL DEFAULT 0,
            total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            first_token_sum_ms REAL NOT NULL DEFAULT 0,
            first_token_max_ms REAL NOT NULL DEFAULT 0,
            first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_request_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_success_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_non_success_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_success_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_non_success_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_failure_cost REAL NOT NULL DEFAULT 0,
            activity_v2_non_success_cost REAL NOT NULL DEFAULT 0,
            activity_v2_cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_cost REAL NOT NULL DEFAULT 0,
            activity_v2_first_response_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_first_response_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_first_token_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_max_ms REAL NOT NULL DEFAULT 0,
            activity_v2_first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            activity_v2_total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            activity_v2_last_invocation_at TEXT,
            activity_v2_latest_unkeyed_conversation_at TEXT,
            activity_v2_latest_first_response_at TEXT,
            activity_v2_latest_first_response_ms REAL,
            activity_v2_latest_total_latency_at TEXT,
            activity_v2_latest_total_latency_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_stats_hourly table existence")?;
    Ok(())
}

async fn ensure_account_stats_hourly_columns(
    pool: &Pool<Sqlite>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    let upstream_account_stats_hourly_columns =
        load_sqlite_table_columns(pool, "upstream_account_stats_hourly").await?;
    state.added_upstream_account_stats_columns = false;
    ensure_account_stats_hourly_base_columns(pool, &upstream_account_stats_hourly_columns, state)
        .await?;
    ensure_account_stats_hourly_activity_columns(pool, &upstream_account_stats_hourly_columns)
        .await?;
    state.upstream_account_stats_hourly_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await
            .context("failed to count upstream_account_stats_hourly rows")?;
    Ok(())
}

async fn ensure_account_stats_hourly_base_columns(
    pool: &Pool<Sqlite>,
    existing_columns: &std::collections::HashSet<String>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    for (column, ty) in [
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !existing_columns.contains(column) {
            state.added_upstream_account_stats_columns = true;
            let statement =
                format!("ALTER TABLE upstream_account_stats_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_hourly column {column}")
                })?;
        }
    }
    Ok(())
}

async fn ensure_account_stats_hourly_activity_columns(
    pool: &Pool<Sqlite>,
    existing_columns: &std::collections::HashSet<String>,
) -> Result<()> {
    for (column, ty) in [
        ("activity_v2_request_count", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_success_count", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_failure_count", "INTEGER NOT NULL DEFAULT 0"),
        (
            "activity_v2_non_success_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_total_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_success_tokens", "INTEGER NOT NULL DEFAULT 0"),
        (
            "activity_v2_non_success_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_failure_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("activity_v2_failure_cost", "REAL NOT NULL DEFAULT 0"),
        ("activity_v2_non_success_cost", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_cache_input_tokens",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_total_cost", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_first_response_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_first_response_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_first_token_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("activity_v2_first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("activity_v2_first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "activity_v2_first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
        (
            "activity_v2_total_latency_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "activity_v2_total_latency_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        ("activity_v2_last_invocation_at", "TEXT"),
        ("activity_v2_latest_unkeyed_conversation_at", "TEXT"),
        ("activity_v2_latest_first_response_at", "TEXT"),
        ("activity_v2_latest_first_response_ms", "REAL"),
        ("activity_v2_latest_total_latency_at", "TEXT"),
        ("activity_v2_latest_total_latency_ms", "REAL"),
    ] {
        if !existing_columns.contains(column) {
            let statement =
                format!("ALTER TABLE upstream_account_stats_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_hourly column {column}")
                })?;
        }
    }
    Ok(())
}

async fn ensure_account_stats_hourly_indexes(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_hourly_account_bucket
        ON upstream_account_stats_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_hourly_source_account_bucket
        ON upstream_account_stats_hourly (source, upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_hourly_source_account_bucket")?;
    Ok(())
}

async fn ensure_account_stats_minute_table(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_account_stats_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            total_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            in_flight_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0,
            non_success_cost REAL NOT NULL DEFAULT 0,
            total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
            total_latency_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_token_sample_count INTEGER NOT NULL DEFAULT 0,
            first_token_sum_ms REAL NOT NULL DEFAULT 0,
            first_token_max_ms REAL NOT NULL DEFAULT 0,
            first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_account_stats_minute table existence")?;
    Ok(())
}
