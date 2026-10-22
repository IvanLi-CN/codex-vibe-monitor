use super::compatibility::ensure_column_with_definition;
use super::*;

pub(super) async fn ensure_summary_coverage_schema(pool: &Pool<Sqlite>) -> Result<()> {
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
        BEFORE UPDATE OF status, summary_source_kind, sha256 ON archive_batches
        WHEN NEW.dataset = 'codex_invocations'
          AND NEW.status = 'completed'
          AND NEW.summary_source_kind = 'authoritative' AND TRIM(COALESCE(NEW.sha256, '')) <> '' AND OLD.replacement_staged_path IS NULL
          AND (
              OLD.status <> 'completed'
              OR OLD.summary_source_kind IS NULL
              OR OLD.summary_source_kind <> 'authoritative'
              OR OLD.sha256 IS NOT NEW.sha256
          )
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
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_repair_progress (
            scope TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            cursor_stale_rank INTEGER NOT NULL DEFAULT -1,
            cursor_month_key TEXT NOT NULL DEFAULT '',
            cursor_created_at TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_repair_progress table existence")?;
    for (column, definition) in [
        ("cursor_stale_rank", "INTEGER NOT NULL DEFAULT -1"),
        ("cursor_month_key", "TEXT NOT NULL DEFAULT ''"),
        ("cursor_created_at", "TEXT NOT NULL DEFAULT ''"),
    ] {
        ensure_column_with_definition(
            pool,
            "hourly_rollup_archive_repair_progress",
            column,
            definition,
        )
        .await
        .with_context(|| {
            format!("failed to ensure hourly rollup archive repair cursor column {column}")
        })?;
    }
    sqlx::query(
        "UPDATE hourly_rollup_archive_repair_progress \
         SET cursor_stale_rank = -1 \
         WHERE cursor_id = 0 \
           AND cursor_month_key = '' AND cursor_created_at = ''",
    )
    .execute(pool)
    .await
    .context("failed to normalize the initial hourly rollup archive repair cursor")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_repair_deferred (
            scope TEXT NOT NULL,
            archive_id INTEGER NOT NULL,
            stale_rank INTEGER NOT NULL,
            month_key TEXT NOT NULL,
            created_at TEXT NOT NULL,
            file_path TEXT NOT NULL,
            deferred_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%f', 'now')),
            PRIMARY KEY (scope, archive_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly rollup archive deferred repair table existence")?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_hourly_rollup_archive_repair_deferred_order \
         ON hourly_rollup_archive_repair_deferred (scope, deferred_at, archive_id)",
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly rollup archive deferred repair index")?;

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
        CREATE TABLE IF NOT EXISTS hourly_rollup_repair_seen_invocation_ids (
            dataset TEXT NOT NULL,
            invocation_id INTEGER NOT NULL,
            PRIMARY KEY (dataset, invocation_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_repair_seen_invocation_ids table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_repair_cleared_buckets (
            dataset TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            PRIMARY KEY (dataset, bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_repair_cleared_buckets table existence")?;

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
           AND batches.sha256 IS NOT NULL \
           AND TRIM(batches.sha256) <> '' \
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

pub(super) async fn ensure_summary_coverage_revision_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS summary_coverage_revision (\
            id INTEGER PRIMARY KEY CHECK (id = 1),\
            revision INTEGER NOT NULL DEFAULT 0\
        )",
    )
    .execute(pool)
    .await
    .context("failed to ensure Summary coverage revision table existence")?;
    sqlx::query("INSERT OR IGNORE INTO summary_coverage_revision (id, revision) VALUES (1, 0)")
        .execute(pool)
        .await
        .context("failed to seed Summary coverage revision")?;
    // A pre-existing database has no revision history for this fence. Bump the zero value once
    // so an old checkpoint is rebuilt under the new coverage contract after upgrade.
    sqlx::query("UPDATE summary_coverage_revision SET revision = 1 WHERE id = 1 AND revision = 0")
        .execute(pool)
        .await
        .context("failed to initialize Summary coverage revision")?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS summary_account_coverage_revision (\
            id INTEGER PRIMARY KEY CHECK (id = 1),\
            revision INTEGER NOT NULL DEFAULT 0\
        )",
    )
    .execute(pool)
    .await
    .context("failed to ensure Summary account coverage revision table existence")?;
    sqlx::query(
        "INSERT OR IGNORE INTO summary_account_coverage_revision (id, revision) VALUES (1, 0)",
    )
    .execute(pool)
    .await
    .context("failed to seed Summary account coverage revision")?;
    sqlx::query(
        "UPDATE summary_account_coverage_revision SET revision = 1 WHERE id = 1 AND revision = 0",
    )
    .execute(pool)
    .await
    .context("failed to initialize Summary account coverage revision")?;

    // Older releases treated every live rollup write as a historical coverage change. On a
    // busy service that continuously reset the staged all-time checkpoint before it could
    // finish. Archive materialization already commits a replay marker, while ordinary live
    // progress is fenced by the independent rollup cursor, so retire those broad triggers on
    // both new and upgraded databases.
    for name in [
        "trg_summary_coverage_revision_invocation_rollup_insert",
        "trg_summary_coverage_revision_invocation_rollup_update",
        "trg_summary_coverage_revision_invocation_rollup_delete",
        "trg_summary_account_coverage_revision_account_rollup_insert",
        "trg_summary_account_coverage_revision_account_rollup_update",
        "trg_summary_account_coverage_revision_account_rollup_delete",
        "trg_summary_account_coverage_revision_usage_rollup_insert",
        "trg_summary_account_coverage_revision_usage_rollup_update",
        "trg_summary_account_coverage_revision_usage_rollup_delete",
        "trg_summary_coverage_revision_snapshot_insert",
        "trg_summary_coverage_revision_snapshot_update",
        "trg_summary_coverage_revision_snapshot_delete",
        "trg_summary_coverage_revision_proof_insert",
        "trg_summary_coverage_revision_proof_update",
        "trg_summary_coverage_revision_proof_delete",
        "trg_summary_account_coverage_revision_snapshot_insert",
        "trg_summary_account_coverage_revision_snapshot_update",
        "trg_summary_account_coverage_revision_snapshot_delete",
        "trg_summary_account_coverage_revision_proof_insert",
        "trg_summary_account_coverage_revision_proof_update",
        "trg_summary_account_coverage_revision_proof_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {name}"))
            .execute(pool)
            .await
            .with_context(|| format!("failed to retire live-tail coverage trigger {name}"))?;
    }

    for (name, table, predicate) in [
        (
            "archive_insert",
            "archive_batches",
            "WHEN NEW.dataset = 'codex_invocations'",
        ),
        (
            "archive_update",
            "archive_batches",
            "WHEN OLD.dataset = 'codex_invocations' OR NEW.dataset = 'codex_invocations'",
        ),
        (
            "archive_delete",
            "archive_batches",
            "WHEN OLD.dataset = 'codex_invocations'",
        ),
        (
            "replay_insert",
            "hourly_rollup_archive_replay",
            "WHEN NEW.dataset = 'codex_invocations'",
        ),
        (
            "replay_update",
            "hourly_rollup_archive_replay",
            "WHEN OLD.dataset = 'codex_invocations' OR NEW.dataset = 'codex_invocations'",
        ),
        (
            "replay_delete",
            "hourly_rollup_archive_replay",
            "WHEN OLD.dataset = 'codex_invocations'",
        ),
        ("proof_insert", "summary_archive_snapshot_v2_proof", ""),
        ("proof_update", "summary_archive_snapshot_v2_proof", ""),
        ("proof_delete", "summary_archive_snapshot_v2_proof", ""),
    ] {
        let event = if name.ends_with("_insert") {
            "INSERT"
        } else if name.ends_with("_update") {
            "UPDATE"
        } else {
            "DELETE"
        };
        let checkpoint_reset = if matches!(
            name,
            "archive_update"
                | "archive_delete"
                | "replay_update"
                | "replay_delete"
                | "proof_update"
                | "proof_delete"
        ) {
            "UPDATE summary_all_time_projection_checkpoint SET \
               global_manifest_next_id = 0, account_manifest_next_id = 0, \
               global_manifest_complete = 0, account_manifest_complete = 0, \
               updated_at = datetime('now') WHERE scope = 'all';"
        } else {
            ""
        };
        let trigger = format!(
            "CREATE TRIGGER IF NOT EXISTS trg_summary_coverage_revision_{name} \
             AFTER {event} ON {table} {predicate} BEGIN \
               UPDATE summary_coverage_revision SET revision = revision + 1 WHERE id = 1; \
               {checkpoint_reset} \
             END",
        );
        sqlx::query(&trigger).execute(pool).await.with_context(|| {
            format!("failed to ensure Summary coverage revision trigger {name}")
        })?;
    }

    // A page write invalidates an existing proof for the same manifest.  Page progress itself
    // is not a coverage revision; only the proof revoke/insert pair is.
    for (name, event, old_id, old_sha, new_id, new_sha) in [
        (
            "insert",
            "INSERT",
            "NEW.archive_batch_id",
            "NEW.manifest_sha256",
            "NEW.archive_batch_id",
            "NEW.manifest_sha256",
        ),
        (
            "update",
            "UPDATE",
            "OLD.archive_batch_id",
            "OLD.manifest_sha256",
            "NEW.archive_batch_id",
            "NEW.manifest_sha256",
        ),
        (
            "delete",
            "DELETE",
            "OLD.archive_batch_id",
            "OLD.manifest_sha256",
            "OLD.archive_batch_id",
            "OLD.manifest_sha256",
        ),
    ] {
        let trigger = format!(
            "CREATE TRIGGER IF NOT EXISTS trg_summary_archive_snapshot_page_invalidates_proof_{name} \
             AFTER {event} ON summary_archive_snapshot BEGIN \
               DELETE FROM summary_archive_snapshot_v2_proof \
                WHERE (archive_batch_id = {old_id} AND manifest_sha256 = {old_sha}) \
                   OR (archive_batch_id = {new_id} AND manifest_sha256 = {new_sha}); \
               UPDATE summary_coverage_obligation \
                  SET state = 'pending', terminal_reason = NULL, resolved_at = NULL, updated_at = datetime('now') \
                WHERE (archive_batch_id = {old_id} AND manifest_sha256 = {old_sha}) \
                   OR (archive_batch_id = {new_id} AND manifest_sha256 = {new_sha}); \
             END",
        );
        sqlx::query(&trigger).execute(pool).await.with_context(|| {
            format!("failed to ensure Summary Snapshot proof invalidation trigger {name}")
        })?;
    }

    // A manifest identity or completion-state change invalidates its proof as well. This closes
    // the gap where a stale marker could survive a manifest rewrite and suppress recovery.
    for (name, event, old_id, old_sha, new_id, new_sha, predicate) in [
        (
            "update",
            "UPDATE",
            "OLD.id",
            "OLD.sha256",
            "NEW.id",
            "NEW.sha256",
            "WHEN (OLD.dataset = 'codex_invocations' OR NEW.dataset = 'codex_invocations') \
                  AND (OLD.id IS NOT NEW.id OR OLD.sha256 IS NOT NEW.sha256 \
                    OR OLD.dataset IS NOT NEW.dataset OR OLD.status IS NOT NEW.status \
                    OR OLD.month_key IS NOT NEW.month_key OR OLD.day_key IS NOT NEW.day_key \
                    OR OLD.part_key IS NOT NEW.part_key OR OLD.file_path IS NOT NEW.file_path \
                    OR OLD.layout IS NOT NEW.layout OR OLD.codec IS NOT NEW.codec \
                    OR OLD.writer_version IS NOT NEW.writer_version \
                    OR OLD.summary_source_kind IS NOT NEW.summary_source_kind \
                    OR OLD.row_count IS NOT NEW.row_count \
                    OR OLD.coverage_start_at IS NOT NEW.coverage_start_at \
                    OR OLD.coverage_end_at IS NOT NEW.coverage_end_at \
                    OR OLD.coverage_start_epoch IS NOT NEW.coverage_start_epoch \
                    OR OLD.coverage_end_epoch IS NOT NEW.coverage_end_epoch)",
        ),
        (
            "delete",
            "DELETE",
            "OLD.id",
            "OLD.sha256",
            "OLD.id",
            "OLD.sha256",
            "WHEN OLD.dataset = 'codex_invocations'",
        ),
    ] {
        if name == "update" {
            sqlx::query(
                "DROP TRIGGER IF EXISTS trg_summary_archive_manifest_invalidates_proof_update",
            )
            .execute(pool)
            .await
            .context("failed to replace Summary manifest proof invalidation trigger")?;
        }
        let trigger = format!(
            "CREATE TRIGGER IF NOT EXISTS trg_summary_archive_manifest_invalidates_proof_{name} \
             AFTER {event} ON archive_batches {predicate} BEGIN \
               DELETE FROM summary_archive_snapshot_v2_proof \
                WHERE (archive_batch_id = {old_id} AND manifest_sha256 = {old_sha}) \
                   OR (archive_batch_id = {new_id} AND manifest_sha256 = {new_sha}); \
               UPDATE summary_coverage_obligation \
                  SET state = 'pending', terminal_reason = NULL, resolved_at = NULL, updated_at = datetime('now') \
                WHERE (archive_batch_id = {old_id} AND manifest_sha256 = {old_sha}) \
                   OR (archive_batch_id = {new_id} AND manifest_sha256 = {new_sha}); \
             END",
        );
        sqlx::query(&trigger).execute(pool).await.with_context(|| {
            format!("failed to ensure Summary manifest proof invalidation trigger {name}")
        })?;
    }

    // Older builds used the global revision for account-only rollups. Remove the old trigger
    // names before installing the account-specific counterparts below; CREATE IF NOT EXISTS
    // alone would preserve the overly broad invalidation on upgraded databases.
    for name in [
        "account_rollup_insert",
        "account_rollup_update",
        "account_rollup_delete",
        "usage_rollup_insert",
        "usage_rollup_update",
        "usage_rollup_delete",
    ] {
        sqlx::query(&format!(
            "DROP TRIGGER IF EXISTS trg_summary_coverage_revision_{name}"
        ))
        .execute(pool)
        .await
        .with_context(|| format!("failed to retire broad Summary coverage trigger {name}"))?;
    }

    // Account coverage has a narrower invalidation set than global coverage. Archive manifests,
    // replay markers and verified V2 pages can change historical account aggregates. Live
    // account and usage rollups remain represented by the independent account live-tail cursor.
    for (name, table, predicate) in [
        (
            "archive_insert",
            "archive_batches",
            "WHEN NEW.dataset = 'codex_invocations'",
        ),
        (
            "archive_update",
            "archive_batches",
            "WHEN OLD.dataset = 'codex_invocations' OR NEW.dataset = 'codex_invocations'",
        ),
        (
            "archive_delete",
            "archive_batches",
            "WHEN OLD.dataset = 'codex_invocations'",
        ),
        (
            "replay_insert",
            "hourly_rollup_archive_replay",
            "WHEN NEW.dataset = 'codex_invocations'",
        ),
        (
            "replay_update",
            "hourly_rollup_archive_replay",
            "WHEN OLD.dataset = 'codex_invocations' OR NEW.dataset = 'codex_invocations'",
        ),
        (
            "replay_delete",
            "hourly_rollup_archive_replay",
            "WHEN OLD.dataset = 'codex_invocations'",
        ),
        ("proof_insert", "summary_archive_snapshot_v2_proof", ""),
        ("proof_update", "summary_archive_snapshot_v2_proof", ""),
        ("proof_delete", "summary_archive_snapshot_v2_proof", ""),
    ] {
        let event = if name.ends_with("_insert") {
            "INSERT"
        } else if name.ends_with("_update") {
            "UPDATE"
        } else {
            "DELETE"
        };
        let trigger = format!(
            "CREATE TRIGGER IF NOT EXISTS trg_summary_account_coverage_revision_{name} \
             AFTER {event} ON {table} {predicate} BEGIN \
               UPDATE summary_account_coverage_revision SET revision = revision + 1 WHERE id = 1; \
             END",
        );
        sqlx::query(&trigger).execute(pool).await.with_context(|| {
            format!("failed to ensure Summary account coverage revision trigger {name}")
        })?;
    }
    Ok(())
}
