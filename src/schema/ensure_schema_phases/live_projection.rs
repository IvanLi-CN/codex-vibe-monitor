use super::*;

pub(super) async fn ensure_schema_live_projection(pool: &Pool<Sqlite>) -> Result<()> {
    let (invocation_existed, prompt_cache_existed) = ensure_live_projection_tables(pool).await?;
    refresh_live_projection(pool, invocation_existed, prompt_cache_existed).await?;
    ensure_quota_and_archive_tables(pool).await?;
    Ok(())
}

async fn ensure_live_projection_tables(pool: &Pool<Sqlite>) -> Result<(bool, bool)> {
    let invocation_in_progress_live_existed =
        schema_sqlite_table_exists(pool, "invocation_in_progress_live").await?;
    let prompt_cache_working_set_live_existed =
        schema_sqlite_table_exists(pool, "prompt_cache_working_set_live").await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_in_progress_live (
            invocation_id INTEGER PRIMARY KEY,
            source TEXT NOT NULL,
            upstream_account_id INTEGER,
            prompt_cache_key TEXT,
            is_retry_after_failure_all INTEGER NOT NULL DEFAULT 0,
            is_retry_after_failure_proxy_only INTEGER NOT NULL DEFAULT 0,
            is_retry_after_failure_account_all INTEGER NOT NULL DEFAULT 0,
            is_retry_after_failure_account_proxy_only INTEGER NOT NULL DEFAULT 0,
            upstream_ttfb_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_in_progress_live table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_in_progress_live_source_account
        ON invocation_in_progress_live (source, upstream_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_in_progress_live_source_account")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_in_progress_live_prompt_cache_key
        ON invocation_in_progress_live (prompt_cache_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_in_progress_live_prompt_cache_key")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_working_set_live (
            prompt_cache_key TEXT PRIMARY KEY,
            source_scope_all INTEGER NOT NULL DEFAULT 1,
            source_scope_proxy_only INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            last_activity_at TEXT NOT NULL,
            last_terminal_at TEXT,
            last_in_flight_at TEXT,
            sort_anchor_at TEXT NOT NULL,
            request_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost REAL NOT NULL DEFAULT 0.0,
            proxy_created_at TEXT,
            proxy_last_activity_at TEXT,
            proxy_last_terminal_at TEXT,
            proxy_last_in_flight_at TEXT,
            proxy_sort_anchor_at TEXT,
            proxy_request_count INTEGER NOT NULL DEFAULT 0,
            proxy_total_tokens INTEGER NOT NULL DEFAULT 0,
            proxy_total_cost REAL NOT NULL DEFAULT 0.0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_working_set_live table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_working_set_live_sort_anchor
        ON prompt_cache_working_set_live (sort_anchor_at DESC, created_at DESC, prompt_cache_key DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_prompt_cache_working_set_live_sort_anchor")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_working_set_live_proxy_sort_anchor
        ON prompt_cache_working_set_live (source_scope_proxy_only, sort_anchor_at DESC, created_at DESC, prompt_cache_key DESC)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_prompt_cache_working_set_live_proxy_sort_anchor")?;
    Ok((
        invocation_in_progress_live_existed,
        prompt_cache_working_set_live_existed,
    ))
}

async fn refresh_live_projection(
    pool: &Pool<Sqlite>,
    invocation_existed: bool,
    prompt_cache_existed: bool,
) -> Result<()> {
    let live_projection_objects_existed =
        invocation_live_projection_objects_exist(pool, invocation_existed, prompt_cache_existed)
            .await?;
    let mut live_projection_refresh_completed =
        schema_refresh_completed(pool, INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME).await?;
    if !live_projection_refresh_completed && live_projection_objects_existed {
        record_schema_refresh_completion(pool, INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME)
            .await?;
        live_projection_refresh_completed = true;
    }
    let refresh_live_projection =
        !live_projection_refresh_completed || !live_projection_objects_existed;
    if refresh_live_projection {
        rebuild_invocation_in_progress_live_triggers(pool)
            .await
            .context("failed to rebuild invocation_in_progress_live triggers")?;
        rebuild_prompt_cache_working_set_live_triggers(pool)
            .await
            .context("failed to rebuild prompt cache working set triggers")?;
        rebuild_invocation_in_progress_live_table(pool)
            .await
            .context("failed to rebuild invocation_in_progress_live table")?;
        rebuild_prompt_cache_working_set_live_table(pool)
            .await
            .context("failed to rebuild prompt_cache_working_set_live table")?;
        record_schema_refresh_completion(pool, INVOCATION_LIVE_PROJECTION_REFRESH_MIGRATION_NAME)
            .await?;
    }
    Ok(())
}

async fn ensure_quota_and_archive_tables(pool: &Pool<Sqlite>) -> Result<()> {
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
            cleanup_source_safe_start_date TEXT,
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            coverage_start_epoch INTEGER,
            coverage_end_epoch INTEGER,
            archive_expires_at TEXT,
            summary_source_kind TEXT NOT NULL DEFAULT 'unknown',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batches table existence")?;
    Ok(())
}
