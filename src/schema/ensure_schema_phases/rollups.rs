async fn ensure_schema_rollups(pool: &Pool<Sqlite>) -> Result<(bool, bool)> {
    let invocation_rollup_hourly_columns =
        load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    let has_existing_invocation_rollup_hourly_rows =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM invocation_rollup_hourly)")
            .fetch_one(pool)
            .await?
            != 0;
    let mut added_invocation_rollup_hourly_columns = false;
    for (column, ty) in [
        ("terminal_count", "INTEGER NOT NULL DEFAULT 0"),
        ("terminal_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("terminal_cost", "REAL NOT NULL DEFAULT 0"),
        ("terminal_proof_complete", "INTEGER NOT NULL DEFAULT 0"),
        ("cache_input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("input_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("output_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reasoning_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("non_success_cost", "REAL NOT NULL DEFAULT 0"),
        ("total_latency_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("total_latency_sum_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_response_byte_total_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_max_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
        ("first_token_sample_count", "INTEGER NOT NULL DEFAULT 0"),
        ("first_token_sum_ms", "REAL NOT NULL DEFAULT 0"),
        ("first_token_max_ms", "REAL NOT NULL DEFAULT 0"),
        (
            "first_token_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !invocation_rollup_hourly_columns.contains(column) {
            added_invocation_rollup_hourly_columns = true;
            let statement =
                format!("ALTER TABLE invocation_rollup_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add invocation_rollup_hourly column {column}")
                })?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_hourly_source_bucket
        ON invocation_rollup_hourly (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_hourly_source_bucket")?;
    if has_existing_invocation_rollup_hourly_rows {
        // The state boundary is the durable completion marker. It must not depend on whether
        // this invocation added columns: a process can stop after the final ALTER TABLE and
        // before this bootstrap runs.
        ensure_long_term_stats_schema(pool).await?;
        let integrity_source_start_date = sqlx::query_scalar::<_, Option<String>>(
            "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
        )
        .fetch_optional(pool)
        .await?
        .flatten();
        if integrity_source_start_date.is_none() {
            crate::long_term_stats::bootstrap_long_term_integrity_source_boundary_for_legacy_rollups(pool)
                .await?;
        }
    }
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_failure_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            failure_class TEXT NOT NULL,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            error_category TEXT NOT NULL,
            failure_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, failure_class, is_actionable, error_category)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_failure_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_failure_rollup_hourly_bucket
        ON invocation_failure_rollup_hourly (bucket_start_epoch, source, failure_class)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_failure_rollup_hourly_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_perf_stage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            stage TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            sum_ms REAL NOT NULL,
            max_ms REAL NOT NULL,
            histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, stage)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_perf_stage_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_proxy_perf_stage_hourly_stage_bucket
        ON proxy_perf_stage_hourly (stage, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_proxy_perf_stage_hourly_stage_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_rollup_hourly_key_bucket
        ON prompt_cache_rollup_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_rollup_hourly_key_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_upstream_account_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            upstream_account_name TEXT,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_upstream_account_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_upstream_account_hourly_key_bucket
        ON prompt_cache_upstream_account_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_upstream_account_hourly_key_bucket")?;

    // These tables intentionally keep only the key identity needed to calculate
    // parallel-work averages. They are not a second copy of invocation history.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_minute_key_rollup (
            minute_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            PRIMARY KEY (minute_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_minute_key_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_minute_key_rollup_source_minute
        ON parallel_work_minute_key_rollup (source, minute_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work minute source range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_upstream_account_minute_key_rollup (
            minute_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            PRIMARY KEY (minute_start_epoch, source, upstream_account_id, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_upstream_account_minute_key_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_account_minute_key_rollup_account_range
        ON parallel_work_upstream_account_minute_key_rollup (upstream_account_id, minute_start_epoch, source)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work account minute range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_hourly_rollup (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            active_minute_count INTEGER NOT NULL,
            parallel_count_sum INTEGER NOT NULL,
            PRIMARY KEY (hour_start_epoch, source_scope)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_hourly_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_hourly_rollup_scope_range
        ON parallel_work_hourly_rollup (source_scope, hour_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work hourly scope range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_upstream_account_hourly_rollup (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            upstream_account_id INTEGER NOT NULL,
            active_minute_count INTEGER NOT NULL,
            parallel_count_sum INTEGER NOT NULL,
            PRIMARY KEY (hour_start_epoch, source_scope, upstream_account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_upstream_account_hourly_rollup table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_parallel_work_account_hourly_rollup_account_range
        ON parallel_work_upstream_account_hourly_rollup (upstream_account_id, source_scope, hour_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel-work account hourly range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_hourly_coverage (
            hour_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            minute_keys_complete INTEGER NOT NULL DEFAULT 0,
            hourly_scalar_complete INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (hour_start_epoch, source_scope)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_hourly_coverage table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS parallel_work_rollup_coverage_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            full_detail_start_epoch INTEGER NOT NULL,
            latest_unrecoverable_detail_epoch INTEGER
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure parallel_work_rollup_coverage_state table existence")?;

    ensure_column_with_definition(
        pool,
        "parallel_work_rollup_coverage_state",
        "latest_unrecoverable_detail_epoch",
        "INTEGER",
    )
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_records (
            minute_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL,
            upstream_account_key INTEGER NOT NULL,
            records_json TEXT NOT NULL,
            max_row_id INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_records table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2 (
            minute_start_epoch INTEGER NOT NULL,
            source_scope TEXT NOT NULL CHECK(source_scope IN ('all', 'proxy_only')),
            upstream_account_key INTEGER NOT NULL,
            aggregate_json TEXT NOT NULL,
            total_latency_samples_json TEXT NOT NULL,
            first_byte_samples_json TEXT NOT NULL,
            first_response_byte_total_samples_json TEXT NOT NULL,
            first_token_samples_json TEXT NOT NULL,
            max_row_id INTEGER NOT NULL DEFAULT 0,
            coverage_state TEXT NOT NULL DEFAULT 'warming',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_timeseries_minute_projection_v2_scope_range
        ON timeseries_minute_projection_v2 (source_scope, upstream_account_key, minute_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 scope range index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS timeseries_minute_projection_v2_state (
            consumer TEXT PRIMARY KEY,
            cursor_row_id INTEGER NOT NULL DEFAULT 0,
            last_flush_at TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure timeseries_minute_projection_v2 state table existence")?;

    Ok((
        has_existing_invocation_rollup_hourly_rows,
        added_invocation_rollup_hourly_columns,
    ))
}
