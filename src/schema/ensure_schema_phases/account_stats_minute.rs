async fn ensure_schema_account_stats_minute(
    pool: &Pool<Sqlite>,
    state: &mut SchemaAccountStatsState,
) -> Result<()> {
    let upstream_account_stats_minute_columns =
        load_sqlite_table_columns(pool, "upstream_account_stats_minute").await?;
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
        if !upstream_account_stats_minute_columns.contains(column) {
            state.added_upstream_account_stats_columns = true;
            let statement =
                format!("ALTER TABLE upstream_account_stats_minute ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add upstream_account_stats_minute column {column}")
                })?;
        }
    }
    state.upstream_account_stats_minute_count =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await
            .context("failed to count upstream_account_stats_minute rows")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_minute_account_bucket
        ON upstream_account_stats_minute (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_minute_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_account_stats_minute_source_account_bucket
        ON upstream_account_stats_minute (source, upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_account_stats_minute_source_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_host_network_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_base_url_host TEXT NOT NULL,
            upload_bytes INTEGER NOT NULL DEFAULT 0,
            download_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_host_network_minute table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_host_network_minute_host_bucket
        ON upstream_host_network_minute (upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_host_network_minute_host_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_host_network_minute_source_host_bucket
        ON upstream_host_network_minute (source, upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_host_network_minute_source_host_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_socket_network_minute (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            upstream_base_url_host TEXT NOT NULL,
            upstream_account_id INTEGER,
            upload_bytes INTEGER NOT NULL DEFAULT 0,
            download_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id
            )
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_socket_network_minute table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_account_bucket
        ON upstream_socket_network_minute (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_account_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_source_bucket
        ON upstream_socket_network_minute (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_source_bucket")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_socket_network_minute_host_bucket
        ON upstream_socket_network_minute (upstream_base_url_host, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_socket_network_minute_host_bucket")?;

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
        CREATE TABLE IF NOT EXISTS pool_upstream_node_health_archive (
            archive_file_path TEXT NOT NULL,
            archived_row_id INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            proxy_binding_key_snapshot TEXT NOT NULL,
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_file_path, archived_row_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_node_health_archive table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_occurred_at_binding
        ON pool_upstream_node_health_archive (occurred_at, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_archive_occurred_at_binding")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_file
        ON pool_upstream_node_health_archive (archive_file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_archive_file")?;

    let hourly_archive_sql = pool_upstream_node_health_hourly_archive_create_sql(
        "pool_upstream_node_health_hourly_archive",
    );
    sqlx::query(&hourly_archive_sql)
        .execute(pool)
        .await
        .context("failed to ensure pool_upstream_node_health_hourly_archive table existence")?;

    let hourly_archive_columns =
        load_sqlite_table_columns(pool, "pool_upstream_node_health_hourly_archive").await?;
    if !hourly_archive_columns.contains("archive_identity")
        || !hourly_archive_columns.contains("archive_batch_id")
    {
        migrate_pool_upstream_node_health_hourly_archive_identity(pool).await?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_bucket_binding
        ON pool_upstream_node_health_hourly_archive (bucket_start_epoch, proxy_binding_key_snapshot)
        "#,
    )
    .execute(pool)
    .await
    .context(
        "failed to ensure index idx_pool_upstream_node_health_hourly_archive_bucket_binding",
    )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_file
        ON pool_upstream_node_health_hourly_archive (archive_file_path)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_hourly_archive_file")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_batch
        ON pool_upstream_node_health_hourly_archive (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_node_health_hourly_archive_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            archive_sha256 TEXT,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_replay table existence")?;

    // SQLite leaves an existing table untouched for CREATE TABLE IF NOT EXISTS. Upgrade the
    // pre-identity replay table before any startup backfill reads or writes archive_sha256.
    let hourly_rollup_archive_replay_columns =
        load_sqlite_table_columns(pool, "hourly_rollup_archive_replay").await?;
    if !hourly_rollup_archive_replay_columns.contains("archive_sha256") {
        sqlx::query("ALTER TABLE hourly_rollup_archive_replay ADD COLUMN archive_sha256 TEXT")
            .execute(pool)
            .await
            .context("failed to add hourly rollup archive replay identity column")?;
    }

    // An authoritative invocation archive becomes Summary-visible only after its bounded source
    // coverage and the three Summary rollup proofs commit in the same transaction. Live-detail
    // mirrors never enter Summary source coverage and therefore do not require these proofs.
    let mut archive_guard_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin authoritative invocation archive guard refresh")?;
    sqlx::query(
        "DROP TRIGGER IF EXISTS trg_insert_authoritative_invocation_archive_requires_summary_proof",
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to replace authoritative invocation archive insert guard")?;
    sqlx::query(
        "DROP TRIGGER IF EXISTS trg_update_authoritative_invocation_archive_requires_summary_proof",
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to replace authoritative invocation archive update guard")?;
    sqlx::query(
        r#"
        CREATE TRIGGER trg_insert_authoritative_invocation_archive_requires_summary_proof
        BEFORE INSERT ON archive_batches
        WHEN NEW.dataset = 'codex_invocations'
          AND NEW.status = 'completed'
          AND NEW.summary_source_kind = 'authoritative'
          AND (
              NEW.coverage_start_at IS NULL
              OR NEW.coverage_end_at IS NULL
              OR NEW.historical_rollups_materialized_at IS NULL
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'invocation_rollup_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_stats_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_usage_breakdown_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
          )
        BEGIN
            SELECT RAISE(ABORT, 'completed codex_invocations archive requires Summary publication proof');
        END
        "#,
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to ensure authoritative invocation archive insert guard")?;
    sqlx::query(
        r#"
        CREATE TRIGGER trg_update_authoritative_invocation_archive_requires_summary_proof
        BEFORE UPDATE OF status, summary_source_kind ON archive_batches
        WHEN NEW.dataset = 'codex_invocations'
          AND NEW.status = 'completed'
          AND NEW.summary_source_kind = 'authoritative'
          AND (OLD.status <> 'completed' OR OLD.summary_source_kind <> 'authoritative')
          AND (
              NEW.coverage_start_at IS NULL
              OR NEW.coverage_end_at IS NULL
              OR NEW.historical_rollups_materialized_at IS NULL
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'invocation_rollup_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_stats_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
              OR NOT EXISTS (
                  SELECT 1 FROM hourly_rollup_archive_replay AS replay
                  WHERE replay.target = 'upstream_account_usage_breakdown_hourly'
                    AND replay.dataset = NEW.dataset
                    AND replay.file_path = NEW.file_path
                    AND replay.archive_sha256 = NEW.sha256
              )
          )
        BEGIN
            SELECT RAISE(ABORT, 'completed codex_invocations archive requires Summary publication proof');
        END
        "#,
    )
    .execute(archive_guard_trigger_tx.as_mut())
    .await
    .context("failed to ensure authoritative invocation archive update guard")?;
    archive_guard_trigger_tx
        .commit()
        .await
        .context("failed to commit authoritative invocation archive guard refresh")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_progress (
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_progress table existence")?;

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
        CREATE TABLE IF NOT EXISTS summary_all_time_coverage_checkpoint (
            scope TEXT PRIMARY KEY,
            manifest_high_watermark_id INTEGER NOT NULL,
            next_manifest_id INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_all_time_coverage_checkpoint table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_source_change_journal (
            cursor INTEGER PRIMARY KEY AUTOINCREMENT,
            descriptor_version INTEGER NOT NULL,
            source_kind TEXT NOT NULL,
            source_revision INTEGER NOT NULL,
            first_row_id INTEGER NOT NULL,
            last_row_id INTEGER NOT NULL,
            occurred_start TEXT NOT NULL,
            occurred_end TEXT NOT NULL,
            descriptor_json TEXT NOT NULL,
            descriptor_bytes INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_source_change_journal table existence")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_summary_source_change_journal_revision \
         ON summary_source_change_journal (source_revision, cursor)",
    )
    .execute(pool)
    .await
    .context("failed to ensure summary source change journal revision index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_source_change_compaction_proof (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            first_cursor INTEGER NOT NULL,
            last_cursor INTEGER NOT NULL,
            proof_kind TEXT NOT NULL,
            proof_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_source_change_compaction_proof table existence")?;
    // Existing installations may have the original proof table without the retained cursor;
    // upgrade it before durable-tail recovery reads the compaction boundary.
    let compaction_proof_columns =
        load_sqlite_table_columns(pool, "summary_source_change_compaction_proof").await?;
    if !compaction_proof_columns.contains("retained_after_cursor") {
        sqlx::query(
            "ALTER TABLE summary_source_change_compaction_proof \
             ADD COLUMN retained_after_cursor INTEGER NOT NULL DEFAULT 0",
        )
        .execute(pool)
        .await
        .context("failed to add summary source compaction retained cursor")?;
    }
    // Older proofs stored the retained boundary only in proof_json. Backfill the typed column
    // before durable-tail recovery reads it; otherwise an upgraded process could treat a
    // compacted prefix as a complete journal and advance past an unproven gap.
    sqlx::query(
        "UPDATE summary_source_change_compaction_proof \
         SET retained_after_cursor = CAST(json_extract(proof_json, '$.retainedAfterCursor') AS INTEGER) \
         WHERE retained_after_cursor = 0 \
           AND json_valid(proof_json) \
           AND COALESCE(json_extract(proof_json, '$.retainedAfterCursor'), 0) > 0",
    )
    .execute(pool)
    .await
    .context("failed to backfill summary source compaction retained cursor")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_source_change_cursor (
            scope TEXT PRIMARY KEY,
            cursor INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_source_change_cursor table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_live_tail_reconciliation_checkpoint (
            scope TEXT PRIMARY KEY,
            format_version INTEGER NOT NULL DEFAULT 1,
            recovery_epoch INTEGER NOT NULL DEFAULT 0,
            base_projection_revision INTEGER NOT NULL DEFAULT 0,
            target_terminal_watermark INTEGER NOT NULL DEFAULT 0,
            target_source_cursor INTEGER NOT NULL DEFAULT 0,
            next_source_cursor INTEGER NOT NULL DEFAULT 0,
            state TEXT NOT NULL DEFAULT 'idle',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure Summary live-tail reconciliation checkpoint table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot (
            archive_batch_id INTEGER NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            page_index INTEGER NOT NULL,
            coverage_start TEXT NOT NULL,
            coverage_end TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            payload BLOB NOT NULL,
            payload_bytes INTEGER NOT NULL,
            snapshot_sha256 TEXT NOT NULL,
            format_version INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_batch_id, manifest_sha256, page_index)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_archive_snapshot table existence")?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot",
        "format_version",
        "INTEGER NOT NULL DEFAULT 1",
    )
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_summary_archive_snapshot_manifest \
         ON summary_archive_snapshot (manifest_sha256, archive_batch_id, page_index)",
    )
    .execute(pool)
    .await
    .context("failed to ensure summary archive snapshot manifest index")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot_backfill_checkpoint (
            scope TEXT PRIMARY KEY,
            next_archive_batch_id INTEGER NOT NULL DEFAULT 0,
            manifest_high_watermark_id INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_archive_snapshot_backfill_checkpoint table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot_backfill_outcome (
            archive_batch_id INTEGER NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            disposition TEXT NOT NULL,
            failure_kind TEXT NOT NULL DEFAULT '',
            next_probe_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            next_occurred_at TEXT,
            cursor_version INTEGER NOT NULL DEFAULT 1,
            retry_attempt INTEGER NOT NULL DEFAULT 0,
            hash_algorithm TEXT NOT NULL DEFAULT '',
            hash_state_version INTEGER NOT NULL DEFAULT 0,
            hash_byte_offset INTEGER NOT NULL DEFAULT 0,
            hash_state BLOB,
            hash_complete_sha256 TEXT,
            source_fingerprint TEXT,
            PRIMARY KEY (archive_batch_id, manifest_sha256)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure summary_archive_snapshot_backfill_outcome table existence")?;

    // Snapshot pages are only in-progress materialization.  A separate proof row is the
    // durable authority used by coverage and cleanup; it is written only after every page has
    // passed manifest, ordering, row-count and semantic validation.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_archive_snapshot_v2_proof (
            archive_batch_id INTEGER NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            page_count INTEGER NOT NULL,
            row_count INTEGER NOT NULL,
            coverage_start TEXT,
            coverage_end TEXT,
            semantic_sha256 TEXT NOT NULL,
            verified_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (archive_batch_id, manifest_sha256)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure Summary Snapshot V2 proof table existence")?;

    // An obligation is independent from an attempt outcome.  In particular, a stale `complete`
    // outcome must not hide a manifest which has no final proof.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS summary_coverage_obligation (
            archive_batch_id INTEGER NOT NULL,
            manifest_sha256 TEXT NOT NULL,
            coverage_start TEXT,
            coverage_end TEXT,
            upstream_account_id INTEGER,
            current_rank_start INTEGER,
            current_rank_end INTEGER,
            state TEXT NOT NULL DEFAULT 'pending',
            terminal_reason TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            resolved_at TEXT,
            PRIMARY KEY (archive_batch_id, manifest_sha256)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure Summary coverage obligation table existence")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_summary_coverage_obligation_window \
         ON summary_coverage_obligation (coverage_end, coverage_start, state)",
    )
    .execute(pool)
    .await
    .context("failed to ensure Summary coverage obligation window index")?;
    // Seed obligations for legacy manifests.  This is idempotent and deliberately excludes
    // identities which already have a verified final proof.
    sqlx::query(
        "INSERT OR IGNORE INTO summary_coverage_obligation \
         (archive_batch_id, manifest_sha256, coverage_start, coverage_end, state) \
         SELECT batches.id, batches.sha256, batches.coverage_start_at, batches.coverage_end_at, 'pending' \
         FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND NOT EXISTS ( \
             SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
             WHERE proof.archive_batch_id = batches.id \
               AND proof.manifest_sha256 = batches.sha256 \
           )",
    )
    .execute(pool)
    .await
    .context("failed to seed Summary coverage obligations")?;
    // Upgrade legacy terminal outcomes into an explicit range-local gap.  Without this bridge a
    // one-year unrecoverable outcome would leave the new obligation looking pending while still
    // suppressing the candidate, recreating the silent zero-candidate state on every restart.
    sqlx::query(
        "UPDATE summary_coverage_obligation AS obligation \
         SET state = 'terminal_gap', \
             terminal_reason = (SELECT outcome.failure_kind \
                                FROM summary_archive_snapshot_backfill_outcome AS outcome \
                                WHERE outcome.archive_batch_id = obligation.archive_batch_id \
                                  AND outcome.manifest_sha256 = obligation.manifest_sha256 \
                                LIMIT 1), \
             resolved_at = NULL, updated_at = datetime('now') \
         WHERE EXISTS ( \
             SELECT 1 FROM summary_archive_snapshot_backfill_outcome AS outcome \
             WHERE outcome.archive_batch_id = obligation.archive_batch_id \
               AND outcome.manifest_sha256 = obligation.manifest_sha256 \
               AND outcome.disposition = 'unavailable' \
               AND outcome.failure_kind IN ( \
                   'verification_failed', 'manifest_sha_mismatch', 'invalid_timestamp', \
                   'row_count_mismatch', 'empty_archive' \
               ) \
         ) \
           AND NOT EXISTS ( \
             SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
             WHERE proof.archive_batch_id = obligation.archive_batch_id \
               AND proof.manifest_sha256 = obligation.manifest_sha256 \
         )",
    )
    .execute(pool)
    .await
    .context("failed to migrate terminal Summary coverage outcomes")?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "next_page_index",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "next_row_id",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "next_occurred_at",
        "TEXT",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "cursor_version",
        "INTEGER NOT NULL DEFAULT 1",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "retry_attempt",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "hash_algorithm",
        "TEXT NOT NULL DEFAULT ''",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "hash_state_version",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "hash_byte_offset",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "hash_state",
        "BLOB",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "hash_complete_sha256",
        "TEXT",
    )
    .await?;
    ensure_column_with_definition(
        pool,
        "summary_archive_snapshot_backfill_outcome",
        "source_fingerprint",
        "TEXT",
    )
    .await?;

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
