use super::*;

pub(crate) async fn ensure_summary_coverage_revision_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_summary_revision_tables(pool).await?;
    retire_legacy_summary_revision_triggers(pool).await?;
    ensure_global_summary_revision_triggers(pool).await?;
    ensure_snapshot_page_proof_triggers(pool).await?;
    ensure_manifest_proof_triggers(pool).await?;
    retire_broad_account_revision_triggers(pool).await?;
    ensure_account_summary_revision_triggers(pool).await?;
    Ok(())
}

async fn ensure_summary_revision_tables(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn retire_legacy_summary_revision_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_global_summary_revision_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_snapshot_page_proof_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_manifest_proof_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn retire_broad_account_revision_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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
    Ok(())
}

async fn ensure_account_summary_revision_triggers(pool: &Pool<Sqlite>) -> Result<()> {
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

pub(crate) async fn ensure_proxy_raw_payload_blob_link_schema(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_raw_link_tables(pool).await?;
    ensure_raw_link_triggers(pool).await?;
    seed_legacy_proxy_raw_payload_blob_links(pool).await?;
    Ok(())
}

async fn ensure_raw_link_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_raw_payload_blobs (
            raw_path TEXT PRIMARY KEY,
            storage_codec TEXT NOT NULL DEFAULT 'identity',
            logical_size_bytes INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blobs")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_raw_payload_blob_links (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            raw_path TEXT NOT NULL,
            owner_kind TEXT NOT NULL CHECK(owner_kind IN ('invocation', 'attempt')),
            owner_id INTEGER NOT NULL,
            raw_role TEXT NOT NULL CHECK(raw_role IN ('request', 'response')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(raw_path, owner_kind, owner_id, raw_role),
            FOREIGN KEY(raw_path) REFERENCES proxy_raw_payload_blobs(raw_path) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blob links")?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_proxy_raw_payload_blob_links_path ON proxy_raw_payload_blob_links (raw_path)",
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blob link path index")?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_raw_payload_blob_link_migrations (
            migration_name TEXT PRIMARY KEY,
            completed_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy raw payload blob link migrations")?;
    Ok(())
}

async fn ensure_raw_link_triggers(pool: &Pool<Sqlite>) -> Result<()> {
    let mut raw_blob_trigger_tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin proxy raw blob trigger refresh")?;
    for trigger in [
        "proxy_raw_blob_link_invocation_insert",
        "proxy_raw_blob_link_invocation_update",
        "proxy_raw_blob_link_invocation_delete",
        "proxy_raw_blob_link_attempt_insert",
        "proxy_raw_blob_link_attempt_update",
        "proxy_raw_blob_link_attempt_delete",
        "proxy_raw_blob_prune_unlinked",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger}"))
            .execute(raw_blob_trigger_tx.as_mut())
            .await
            .with_context(|| format!("failed to replace proxy raw blob trigger {trigger}"))?;
    }

    let link_invocation = r#"
        INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes)
        SELECT NEW.request_raw_path, COALESCE(NULLIF(TRIM(NEW.request_raw_codec), ''), 'identity'), COALESCE(NEW.request_raw_size, 0)
        WHERE NEW.request_raw_path IS NOT NULL
        ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now');
        INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role)
        SELECT NEW.request_raw_path, 'invocation', NEW.id, 'request'
        WHERE NEW.request_raw_path IS NOT NULL;
        INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes)
        SELECT NEW.response_raw_path, COALESCE(NULLIF(TRIM(NEW.response_raw_codec), ''), 'identity'), COALESCE(NEW.response_raw_size, 0)
        WHERE NEW.response_raw_path IS NOT NULL
        ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now');
        INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role)
        SELECT NEW.response_raw_path, 'invocation', NEW.id, 'response'
        WHERE NEW.response_raw_path IS NOT NULL;
    "#;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_invocation_insert AFTER INSERT ON codex_invocations BEGIN {link_invocation} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create invocation raw blob insert trigger")?;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_invocation_update AFTER UPDATE OF request_raw_path, request_raw_codec, request_raw_size, response_raw_path, response_raw_codec, response_raw_size ON codex_invocations BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'invocation' AND owner_id = NEW.id; {link_invocation} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create invocation raw blob update trigger")?;
    sqlx::query(
        "CREATE TRIGGER proxy_raw_blob_link_invocation_delete AFTER DELETE ON codex_invocations BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'invocation' AND owner_id = OLD.id; END",
    )
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create invocation raw blob delete trigger")?;

    let link_attempt = r#"
        INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes)
        SELECT NEW.response_raw_path, COALESCE(NULLIF(TRIM(NEW.response_raw_codec), ''), 'identity'), COALESCE(NEW.response_raw_size, 0)
        WHERE NEW.response_raw_path IS NOT NULL
        ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now');
        INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role)
        SELECT NEW.response_raw_path, 'attempt', NEW.id, 'response'
        WHERE NEW.response_raw_path IS NOT NULL;
    "#;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_attempt_insert AFTER INSERT ON pool_upstream_request_attempts BEGIN {link_attempt} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create attempt raw blob insert trigger")?;
    sqlx::query(&format!(
        "CREATE TRIGGER proxy_raw_blob_link_attempt_update AFTER UPDATE OF response_raw_path, response_raw_codec, response_raw_size ON pool_upstream_request_attempts BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'attempt' AND owner_id = NEW.id; {link_attempt} END"
    ))
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create attempt raw blob update trigger")?;
    sqlx::query(
        "CREATE TRIGGER proxy_raw_blob_link_attempt_delete AFTER DELETE ON pool_upstream_request_attempts BEGIN DELETE FROM proxy_raw_payload_blob_links WHERE owner_kind = 'attempt' AND owner_id = OLD.id; END",
    )
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create attempt raw blob delete trigger")?;
    sqlx::query(
        "CREATE TRIGGER proxy_raw_blob_prune_unlinked AFTER DELETE ON proxy_raw_payload_blob_links BEGIN DELETE FROM proxy_raw_payload_blobs WHERE raw_path = OLD.raw_path AND NOT EXISTS (SELECT 1 FROM proxy_raw_payload_blob_links WHERE raw_path = OLD.raw_path); END",
    )
    .execute(raw_blob_trigger_tx.as_mut())
    .await
    .context("failed to create proxy raw blob pruning trigger")?;
    raw_blob_trigger_tx
        .commit()
        .await
        .context("failed to commit proxy raw blob trigger refresh")?;
    Ok(())
}

async fn seed_legacy_proxy_raw_payload_blob_links(pool: &Pool<Sqlite>) -> Result<()> {
    const MIGRATION_NAME: &str = "seed_existing_raw_blob_links_v1";
    let already_seeded = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM proxy_raw_payload_blob_link_migrations WHERE migration_name = ?1)",
    )
    .bind(MIGRATION_NAME)
    .fetch_one(pool)
    .await?
        != 0;
    if already_seeded {
        return Ok(());
    }

    // Existing rows predate the link triggers. Seed both owner types atomically once so an
    // upgraded database retains paired response blobs and inventories attempt-only captures.
    let mut tx = pool.begin().await?;
    for (path_column, codec_column, size_column) in [
        ("request_raw_path", "request_raw_codec", "request_raw_size"),
        (
            "response_raw_path",
            "response_raw_codec",
            "response_raw_size",
        ),
    ] {
        let query = format!(
            "INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes) SELECT {path_column}, COALESCE(NULLIF(TRIM({codec_column}), ''), 'identity'), COALESCE({size_column}, 0) FROM codex_invocations WHERE {path_column} IS NOT NULL ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now')"
        );
        sqlx::query(&query).execute(tx.as_mut()).await?;
    }
    sqlx::query(
        "INSERT INTO proxy_raw_payload_blobs (raw_path, storage_codec, logical_size_bytes) SELECT response_raw_path, COALESCE(NULLIF(TRIM(response_raw_codec), ''), 'identity'), COALESCE(response_raw_size, 0) FROM pool_upstream_request_attempts WHERE response_raw_path IS NOT NULL ON CONFLICT(raw_path) DO UPDATE SET updated_at = datetime('now')",
    )
    .execute(tx.as_mut())
    .await?;
    for (path_column, raw_role) in [
        ("request_raw_path", "request"),
        ("response_raw_path", "response"),
    ] {
        let query = format!(
            "INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role) SELECT {path_column}, 'invocation', id, '{raw_role}' FROM codex_invocations WHERE {path_column} IS NOT NULL"
        );
        sqlx::query(&query).execute(tx.as_mut()).await?;
    }
    sqlx::query(
        "INSERT OR IGNORE INTO proxy_raw_payload_blob_links (raw_path, owner_kind, owner_id, raw_role) SELECT response_raw_path, 'attempt', id, 'response' FROM pool_upstream_request_attempts WHERE response_raw_path IS NOT NULL",
    )
    .execute(tx.as_mut())
    .await?;
    sqlx::query("INSERT INTO proxy_raw_payload_blob_link_migrations (migration_name) VALUES (?1)")
        .bind(MIGRATION_NAME)
        .execute(tx.as_mut())
        .await?;
    tx.commit().await?;
    Ok(())
}
