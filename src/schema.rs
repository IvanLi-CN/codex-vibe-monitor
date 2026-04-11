async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let create_sql = codex_invocations_create_sql("codex_invocations");
    sqlx::query(&create_sql)
        .execute(pool)
        .await
        .context("failed to ensure codex_invocations table existence")?;

    let mut existing = load_sqlite_table_columns(pool, "codex_invocations").await?;
    if existing.contains("raw_expires_at") {
        migrate_codex_invocations_drop_raw_expires_at(pool, &existing).await?;
        existing = load_sqlite_table_columns(pool, "codex_invocations").await?;
    }

    for (column, ty) in [
        ("source", "TEXT NOT NULL DEFAULT 'xy'"),
        ("model", "TEXT"),
        ("input_tokens", "INTEGER"),
        ("output_tokens", "INTEGER"),
        ("cache_input_tokens", "INTEGER"),
        ("reasoning_tokens", "INTEGER"),
        ("total_tokens", "INTEGER"),
        ("cost", "REAL"),
        ("status", "TEXT"),
        ("error_message", "TEXT"),
        ("failure_kind", "TEXT"),
        ("failure_class", "TEXT"),
        ("is_actionable", "INTEGER NOT NULL DEFAULT 0"),
        ("payload", "TEXT"),
        ("cost_estimated", "INTEGER NOT NULL DEFAULT 0"),
        ("price_version", "TEXT"),
        ("request_raw_path", "TEXT"),
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("request_raw_size", "INTEGER"),
        ("request_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("request_raw_truncated_reason", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("detail_level", "TEXT NOT NULL DEFAULT 'full'"),
        ("detail_pruned_at", "TEXT"),
        ("detail_prune_reason", "TEXT"),
        ("t_total_ms", "REAL"),
        ("t_req_read_ms", "REAL"),
        ("t_req_parse_ms", "REAL"),
        ("t_upstream_connect_ms", "REAL"),
        ("t_upstream_ttfb_ms", "REAL"),
        ("t_upstream_stream_ms", "REAL"),
        ("t_resp_parse_ms", "REAL"),
        ("t_persist_ms", "REAL"),
    ] {
        if !existing.contains(column) {
            let statement = format!("ALTER TABLE codex_invocations ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add column {column}"))?;
        }
    }

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET request_raw_codec = CASE
                WHEN request_raw_path IS NOT NULL AND request_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(request_raw_codec), '') = ''
           OR (request_raw_codec = 'identity' AND request_raw_path LIKE '%.gz')
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill codex_invocations request_raw_codec")?;

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET response_raw_codec = CASE
                WHEN response_raw_path IS NOT NULL AND response_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(response_raw_codec), '') = ''
           OR (response_raw_codec = 'identity' AND response_raw_path LIKE '%.gz')
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill codex_invocations response_raw_codec")?;

    // Speed up time-range scans and ordering on the stats endpoints
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at
        ON codex_invocations (occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at")?;

    // Benefit queries that filter by time and status (e.g., error distribution)
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at_status
        ON codex_invocations (occurred_at, status)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at_status")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_source_occurred_at
        ON codex_invocations (source, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_source_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_class_occurred_at
        ON codex_invocations (failure_class, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_class_occurred_at")?;

    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_occurred_at")
        .execute(pool)
        .await
        .context("failed to drop stale idx_codex_invocations_prompt_cache_key_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_model_occurred_at
        ON codex_invocations (model, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_model_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_kind_occurred_at
        ON codex_invocations (failure_kind, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_kind_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_endpoint_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.endpoint') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_endpoint_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_requester_ip_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.requesterIp') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_requester_ip_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_upstream_account_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_upstream_account_occurred_at")?;

    // The records analytics page compares trimmed lowercase text for exact-match filters.
    // Mirror those expressions in dedicated indexes so high-volume searches avoid full index scans.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_model_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(model, '')))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_model_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_kind_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END,
                failure_kind
            ), '')))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_kind_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_endpoint_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_endpoint_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_requester_ip_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_requester_ip_filter_occurred_at")?;

    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at")
        .execute(pool)
        .await
        .context(
            "failed to drop stale idx_codex_invocations_prompt_cache_key_filter_occurred_at",
        )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.promptCacheKey') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_proxy_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                COALESCE(
                    NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), ''),
                    CASE WHEN TRIM(source) != 'proxy' THEN TRIM(source) END
                ),
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_proxy_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_request_raw_pending
        ON codex_invocations (occurred_at, id)
        WHERE request_raw_path IS NOT NULL
          AND request_raw_codec = 'identity'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_request_raw_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_response_raw_pending
        ON codex_invocations (occurred_at, id)
        WHERE response_raw_path IS NOT NULL
          AND response_raw_codec = 'identity'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_response_raw_pending")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS codex_quota_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at TEXT NOT NULL DEFAULT (datetime('now')),
            amount_limit REAL,
            used_amount REAL,
            remaining_amount REAL,
            period TEXT,
            period_reset_time TEXT,
            expire_time TEXT,
            is_active INTEGER,
            total_cost REAL,
            total_requests INTEGER,
            total_tokens INTEGER,
            last_request_time TEXT,
            billing_type TEXT,
            remaining_count INTEGER,
            used_count INTEGER,
            sub_type_name TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure codex_quota_snapshots table existence")?;

    // Speed up latest snapshot lookup
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_quota_snapshots_captured_at
        ON codex_quota_snapshots (captured_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_quota_snapshots_captured_at")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS stats_source_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            period TEXT NOT NULL,
            stats_date TEXT NOT NULL,
            model TEXT,
            requests INTEGER NOT NULL,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_create_tokens INTEGER,
            cache_read_tokens INTEGER,
            all_tokens INTEGER,
            cost_input REAL,
            cost_output REAL,
            cost_cache_write REAL,
            cost_cache_read REAL,
            cost_total REAL,
            raw_response TEXT,
            captured_at TEXT NOT NULL,
            captured_at_epoch INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(source, period, stats_date, model, captured_at_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure stats_source_snapshots table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_stats_source_snapshots_date
        ON stats_source_snapshots (source, period, stats_date, captured_at_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_stats_source_snapshots_date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS stats_source_deltas (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            period TEXT NOT NULL,
            stats_date TEXT NOT NULL,
            captured_at TEXT NOT NULL,
            captured_at_epoch INTEGER NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(source, period, stats_date, captured_at_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure stats_source_deltas table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_stats_source_deltas_epoch
        ON stats_source_deltas (source, period, captured_at_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_stats_source_deltas_epoch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL DEFAULT 'legacy_month',
            codec TEXT NOT NULL DEFAULT 'gzip',
            writer_version TEXT NOT NULL DEFAULT 'legacy_month_v1',
            cleanup_state TEXT NOT NULL DEFAULT 'active',
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            archive_expires_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batches table existence")?;

    let archive_batch_columns = load_sqlite_table_columns(pool, "archive_batches").await?;
    for (column, ty) in [
        ("day_key", "TEXT"),
        ("part_key", "TEXT"),
        ("layout", "TEXT NOT NULL DEFAULT 'legacy_month'"),
        ("codec", "TEXT NOT NULL DEFAULT 'gzip'"),
        ("writer_version", "TEXT NOT NULL DEFAULT 'legacy_month_v1'"),
        ("cleanup_state", "TEXT NOT NULL DEFAULT 'active'"),
        ("superseded_by", "INTEGER"),
        ("coverage_start_at", "TEXT"),
        ("coverage_end_at", "TEXT"),
        ("archive_expires_at", "TEXT"),
        ("upstream_activity_manifest_refreshed_at", "TEXT"),
        ("historical_rollups_materialized_at", "TEXT"),
    ] {
        if !archive_batch_columns.contains(column) {
            let statement = format!("ALTER TABLE archive_batches ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add archive_batches column {column}"))?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_month
        ON archive_batches (dataset, month_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_month")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_layout_day_part
        ON archive_batches (dataset, layout, day_key, part_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_layout_day_part")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_manifest_pending
        ON archive_batches (dataset, status, upstream_activity_manifest_refreshed_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_invocation_manifest_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_rollup_materialization
        ON archive_batches (dataset, status, historical_rollups_materialized_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_rollup_materialization")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batch_upstream_activity (
            archive_batch_id INTEGER NOT NULL,
            account_id INTEGER NOT NULL,
            last_activity_at TEXT NOT NULL,
            PRIMARY KEY (archive_batch_id, account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batch_upstream_activity table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_account_last_activity
        ON archive_batch_upstream_activity (account_id, last_activity_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_account_last_activity")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_batch
        ON archive_batch_upstream_activity (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_materialized_buckets (
            target TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            materialized_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_materialized_buckets table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_hourly_rollup_materialized_buckets_target_bucket
        ON hourly_rollup_materialized_buckets (target, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_hourly_rollup_materialized_buckets_target_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_daily (
            stats_date TEXT NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_daily table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_daily_source_date
        ON invocation_rollup_daily (source, stats_date)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_daily_source_date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_hourly table existence")?;

    let invocation_rollup_hourly_columns =
        load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    let mut added_first_response_byte_total_rollup_columns = false;
    for (column, ty) in [
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
    ] {
        if !invocation_rollup_hourly_columns.contains(column) {
            added_first_response_byte_total_rollup_columns = true;
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
    if added_first_response_byte_total_rollup_columns {
        let rebuilt_rows = backfill_invocation_rollup_hourly_from_sources(pool).await?;
        info!(
            rebuilt_rows,
            "backfilled invocation hourly rollups after adding first-response-byte-total columns"
        );
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

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_sticky_key_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            sticky_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id, sticky_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_sticky_key_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_sticky_key_hourly_account_bucket
        ON upstream_sticky_key_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_sticky_key_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempt_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            latency_sample_count INTEGER NOT NULL DEFAULT 0,
            latency_sum_ms REAL NOT NULL DEFAULT 0,
            latency_max_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempt_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempt_hourly_bucket_proxy
        ON forward_proxy_attempt_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempt_hourly_bucket_proxy")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_replay table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_live_progress (
            dataset TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_live_progress table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'disabled',
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 3,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_model_settings table existence")?;

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN enabled_preset_models_json TEXT
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure enabled_preset_models_json column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'disabled'
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure fast_mode_rewrite_mode column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN preset_models_migrated INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure preset_models_migrated column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN upstream_429_max_retries INTEGER NOT NULL DEFAULT 3
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure upstream_429_max_retries column");
    }

    let default_enabled_models_json = serde_json::to_string(&default_enabled_preset_models())
        .context("failed to serialize default enabled preset models")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries,
            enabled_preset_models_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(DEFAULT_PROXY_MODELS_HIJACK_ENABLED as i64)
    .bind(DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED as i64)
    .bind(i64::from(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES))
    .bind(default_enabled_models_json)
    .execute(pool)
    .await
    .context("failed to ensure default proxy_model_settings row")?;

    ensure_proxy_enabled_models_contains_new_presets(pool)
        .await
        .context("failed to ensure proxy preset models list is up-to-date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_settings_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            catalog_version TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pricing_settings_meta table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_settings_models (
            model TEXT PRIMARY KEY,
            input_per_1m REAL NOT NULL,
            output_per_1m REAL NOT NULL,
            cache_input_per_1m REAL,
            reasoning_per_1m REAL,
            source TEXT NOT NULL DEFAULT 'custom',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pricing_settings_models table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS oauth_bridge_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            installation_seed TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure oauth_bridge_settings table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            proxy_urls_json TEXT NOT NULL DEFAULT '[]',
            subscription_urls_json TEXT NOT NULL DEFAULT '[]',
            subscription_update_interval_secs INTEGER NOT NULL DEFAULT 3600,
            insert_direct INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_settings table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_runtime (
            proxy_key TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL,
            endpoint_url TEXT,
            weight REAL NOT NULL,
            success_ema REAL NOT NULL,
            latency_ema_ms REAL,
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            is_penalized INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_runtime table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_metadata_history (
            proxy_key TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL,
            endpoint_url TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_metadata_history table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            proxy_key TEXT NOT NULL,
            occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            failure_kind TEXT,
            is_probe INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempts table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_request_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            route_mode TEXT NOT NULL,
            sticky_key TEXT,
            upstream_account_id INTEGER,
            upstream_route_key TEXT,
            attempt_index INTEGER NOT NULL,
            distinct_account_index INTEGER NOT NULL,
            same_account_retry_index INTEGER NOT NULL,
            requester_ip TEXT,
            started_at TEXT,
            finished_at TEXT,
            status TEXT NOT NULL,
            phase TEXT,
            http_status INTEGER,
            downstream_http_status INTEGER,
            failure_kind TEXT,
            error_message TEXT,
            downstream_error_message TEXT,
            connect_latency_ms REAL,
            first_byte_latency_ms REAL,
            stream_latency_ms REAL,
            upstream_request_id TEXT,
            compact_support_status TEXT,
            compact_support_reason TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_request_attempts table existence")?;

    let existing_pool_attempt_columns =
        load_sqlite_table_columns(pool, "pool_upstream_request_attempts").await?;
    for (column, ty) in [
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
    ] {
        if !existing_pool_attempt_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add pool_upstream_request_attempts column {column}")
                })?;
        }
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_weight_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            min_weight REAL NOT NULL,
            max_weight REAL NOT NULL,
            avg_weight REAL NOT NULL,
            last_weight REAL NOT NULL,
            last_sample_epoch_us INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_weight_hourly table existence")?;

    let existing_forward_proxy_weight_columns: HashSet<String> =
        sqlx::query("PRAGMA table_info('forward_proxy_weight_hourly')")
            .fetch_all(pool)
            .await
            .context("failed to inspect forward_proxy_weight_hourly schema")?
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();
    if !existing_forward_proxy_weight_columns.contains("last_sample_epoch_us") {
        sqlx::query(
            r#"
            ALTER TABLE forward_proxy_weight_hourly
            ADD COLUMN last_sample_epoch_us INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add last_sample_epoch_us to forward_proxy_weight_hourly")?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempts_proxy_time
        ON forward_proxy_attempts (proxy_key, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempts_proxy_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempts_time_proxy
        ON forward_proxy_attempts (occurred_at, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempts_time_proxy")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_invoke_attempt
        ON pool_upstream_request_attempts (invoke_id, attempt_index)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_invoke_attempt")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_account_occurred_at
        ON pool_upstream_request_attempts (upstream_account_id, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_account_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_sticky_occurred_at
        ON pool_upstream_request_attempts (sticky_key, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_sticky_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_occurred_at
        ON pool_upstream_request_attempts (occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_weight_hourly_time_proxy
        ON forward_proxy_weight_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_weight_hourly_time_proxy")?;

    let default_proxy_urls_json =
        serde_json::to_string(&Vec::<String>::new()).context("serialize default proxy urls")?;
    let default_subscription_urls_json = serde_json::to_string(&Vec::<String>::new())
        .context("serialize default proxy subscription urls")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO forward_proxy_settings (
            id,
            proxy_urls_json,
            subscription_urls_json,
            subscription_update_interval_secs,
            insert_direct
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .bind(default_proxy_urls_json)
    .bind(default_subscription_urls_json)
    .bind(DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS as i64)
    .bind(DEFAULT_FORWARD_PROXY_INSERT_DIRECT as i64)
    .execute(pool)
    .await
    .context("failed to ensure default forward_proxy_settings row")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS startup_backfill_progress (
            task_name TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            next_run_after TEXT,
            zero_update_streak INTEGER NOT NULL DEFAULT 0,
            last_started_at TEXT,
            last_finished_at TEXT,
            last_scanned INTEGER NOT NULL DEFAULT 0,
            last_updated INTEGER NOT NULL DEFAULT 0,
            last_status TEXT NOT NULL DEFAULT 'idle'
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure startup_backfill_progress table existence")?;

    seed_default_pricing_catalog(pool).await?;
    ensure_upstream_accounts_schema(pool).await?;

    Ok(())
}
