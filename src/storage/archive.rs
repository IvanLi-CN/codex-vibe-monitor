//! Transactional archive metadata and Raw-reference boundaries for the traffic module.
//!
//! The adapter in this module owns PostgreSQL archive metadata only. It deliberately does
//! not unlink archive or Raw files and does not publish account-routing side effects. The
//! caller must perform a second, identity-checked file operation after `prune_metadata`.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};

const SHA256_HEX_LENGTH: usize = 64;
const DEFAULT_LEGACY_TABLE: &str = "codex_invocations";
const ARCHIVE_STATUS_COMPLETED: &str = "completed";
const ARCHIVE_CLEANUP_STATE_DELETE_PENDING: &str = "delete_pending";
const HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE: &str = "";
const SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE: &str = "authoritative";
const SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR: &str = "live_mirror";
pub(crate) const INVOCATION_SUMMARY_REPLAY_TARGETS: [&str; 3] = [
    "invocation_rollup_hourly",
    "upstream_account_stats_hourly",
    "upstream_account_usage_breakdown_hourly",
];
// Keep release and prune on the same owner-tuple set. UNION intentionally deduplicates a
// tuple that is present in both the live and archive ledgers during a migration window.
const ACTIVE_RAW_REFERENCE_UNION: &str = "SELECT raw_path, owner_kind, owner_id, raw_role
                                           FROM proxy_raw_payload_blob_links
                                          WHERE raw_path = $1
                                         UNION
                                         SELECT raw_path, owner_kind, owner_id, raw_role
                                           FROM archive_raw_reference_changes
                                          WHERE raw_path = $1
                                            AND action = 'link' AND state = 'active'";

/// PostgreSQL tables owned by the archive adapter. The list is intentionally narrow so the
/// migration engine and account-routing module cannot accidentally become dependencies of it.
pub(crate) const ARCHIVE_STORAGE_TABLES: &[&str] = &[
    "archive_batches",
    "archive_batch_upstream_activity",
    "archive_rollup_markers",
    "hourly_rollup_archive_replay",
    "hourly_rollup_materialized_buckets",
    "pool_upstream_node_health_archive",
    "pool_upstream_node_health_hourly_archive",
    "archive_proofs",
    "proxy_raw_payload_blobs",
    "proxy_raw_payload_blob_links",
    "archive_raw_reference_changes",
    "archive_account_activity_outbox",
    "archive_prune_ledger",
    "retention_prepared_archives",
    "retention_recovery_cursors",
    "retention_raw_reconciliation",
    "account_activity_v2_bucket_repair_watermarks",
];

const ARCHIVE_STORAGE_SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS archive_batches (
        archive_batch_id BIGINT PRIMARY KEY,
        dataset TEXT NOT NULL,
        month_key TEXT NOT NULL,
        day_key TEXT,
        part INTEGER NOT NULL DEFAULT 0,
        file_path TEXT NOT NULL,
        sha256 TEXT NOT NULL,
        source_identity_sha256 TEXT NOT NULL,
        row_count BIGINT NOT NULL,
        status TEXT NOT NULL DEFAULT 'completed',
        layout TEXT NOT NULL,
        codec TEXT NOT NULL,
        writer_version TEXT NOT NULL DEFAULT 'legacy_month_v1',
        cleanup_state TEXT NOT NULL,
        cleanup_source_safe_start_date TEXT,
        replacement_staged_path TEXT,
        superseded_by BIGINT,
        coverage_start_at TEXT,
        coverage_end_at TEXT,
        archive_expires_at TEXT,
        summary_source_kind TEXT NOT NULL DEFAULT 'unknown',
        upstream_activity_manifest_refreshed_at TEXT,
        historical_rollups_materialized_at TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (dataset, month_key, file_path),
        UNIQUE (dataset, file_path)
    )",
    "CREATE TABLE IF NOT EXISTS archive_batch_upstream_activity (
        archive_batch_id BIGINT NOT NULL REFERENCES archive_batches(archive_batch_id),
        account_id BIGINT NOT NULL,
        last_activity_at TEXT NOT NULL,
        PRIMARY KEY (archive_batch_id, account_id)
    )",
    "CREATE TABLE IF NOT EXISTS archive_rollup_markers (
        archive_batch_id BIGINT NOT NULL REFERENCES archive_batches(archive_batch_id),
        target TEXT NOT NULL,
        bucket_start_epoch_ms BIGINT NOT NULL,
        source_identity_sha256 TEXT NOT NULL,
        materialized_rows BIGINT NOT NULL,
        complete BOOLEAN NOT NULL,
        PRIMARY KEY (archive_batch_id, target, bucket_start_epoch_ms)
    )",
    "CREATE TABLE IF NOT EXISTS archive_proofs (
        archive_batch_id BIGINT PRIMARY KEY REFERENCES archive_batches(archive_batch_id),
        manifest_sha256 TEXT NOT NULL,
        page_count BIGINT NOT NULL,
        row_count BIGINT NOT NULL,
        coverage_start_at TEXT,
        coverage_end_at TEXT,
        semantic_sha256 TEXT NOT NULL,
        verified_at_epoch_ms BIGINT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS pool_upstream_node_health_archive (
        archive_file_path TEXT NOT NULL,
        archived_row_id BIGINT NOT NULL,
        occurred_at TEXT NOT NULL,
        proxy_binding_key_snapshot TEXT NOT NULL,
        is_success BIGINT NOT NULL CHECK (is_success IN (0, 1)),
        latency_ms DOUBLE PRECISION,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY (archive_file_path, archived_row_id)
    )",
    "CREATE TABLE IF NOT EXISTS pool_upstream_node_health_hourly_archive (
        archive_identity TEXT NOT NULL,
        archive_batch_id BIGINT REFERENCES archive_batches(archive_batch_id),
        archive_file_path TEXT NOT NULL,
        proxy_binding_key_snapshot TEXT NOT NULL,
        bucket_start_epoch BIGINT NOT NULL,
        success_count BIGINT NOT NULL,
        failure_count BIGINT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY (archive_identity, proxy_binding_key_snapshot, bucket_start_epoch)
    )",
    "CREATE TABLE IF NOT EXISTS proxy_raw_payload_blobs (
        raw_path TEXT PRIMARY KEY,
        storage_codec TEXT NOT NULL DEFAULT 'identity',
        logical_size_bytes BIGINT NOT NULL DEFAULT 0,
        file_identity_sha256 TEXT,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        CHECK (file_identity_sha256 IS NULL OR file_identity_sha256 ~ '^[0-9A-Fa-f]{64}$')
    )",
    "CREATE TABLE IF NOT EXISTS proxy_raw_payload_blob_links (
        id BIGSERIAL PRIMARY KEY,
        raw_path TEXT NOT NULL REFERENCES proxy_raw_payload_blobs(raw_path) ON DELETE CASCADE,
        owner_kind TEXT NOT NULL CHECK (owner_kind IN ('invocation', 'attempt')),
        owner_id BIGINT NOT NULL,
        raw_role TEXT NOT NULL CHECK (raw_role IN ('request', 'response')),
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        UNIQUE (raw_path, owner_kind, owner_id, raw_role)
    )",
    "CREATE TABLE IF NOT EXISTS archive_raw_reference_changes (
        archive_batch_id BIGINT NOT NULL REFERENCES archive_batches(archive_batch_id),
        raw_path TEXT NOT NULL,
        owner_kind TEXT NOT NULL,
        owner_id BIGINT NOT NULL,
        raw_role TEXT NOT NULL,
        action TEXT NOT NULL,
        raw_identity_sha256 TEXT,
        state TEXT NOT NULL,
        PRIMARY KEY (archive_batch_id, raw_path, owner_kind, owner_id, raw_role),
        CHECK (action IN ('link', 'release')),
        CHECK (state IN ('active', 'released')),
        CHECK (owner_kind IN ('invocation', 'attempt')),
        CHECK (raw_role IN ('request', 'response')),
        CHECK (action <> 'link' OR raw_identity_sha256 IS NOT NULL)
    )",
    "CREATE TABLE IF NOT EXISTS archive_account_activity_outbox (
        idempotency_key TEXT PRIMARY KEY,
        archive_batch_id BIGINT NOT NULL REFERENCES archive_batches(archive_batch_id),
        account_id BIGINT NOT NULL,
        last_activity_at TEXT NOT NULL,
        source_revision BIGINT NOT NULL,
        control_generation BIGINT NOT NULL,
        state TEXT NOT NULL,
        acknowledged_generation BIGINT,
        CHECK (state IN ('pending', 'acknowledged'))
    )",
    "CREATE TABLE IF NOT EXISTS archive_prune_ledger (
        archive_batch_id BIGINT PRIMARY KEY,
        file_path TEXT NOT NULL,
        manifest_sha256 TEXT NOT NULL,
        file_identity_sha256 TEXT NOT NULL,
        prepared_at_epoch_ms BIGINT NOT NULL,
        metadata_pruned_at_epoch_ms BIGINT
    )",
    "CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
        target TEXT NOT NULL,
        dataset TEXT NOT NULL,
        file_path TEXT NOT NULL,
        archive_sha256 TEXT,
        replayed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY (target, dataset, file_path)
    )",
    "CREATE TABLE IF NOT EXISTS hourly_rollup_materialized_buckets (
        target TEXT NOT NULL,
        bucket_start_epoch_ms BIGINT NOT NULL,
        source TEXT NOT NULL DEFAULT '',
        materialized_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        PRIMARY KEY (target, bucket_start_epoch_ms, source)
    )",
    "CREATE TABLE IF NOT EXISTS account_activity_v2_bucket_repair_watermarks (
        bucket_start_epoch_ms BIGINT PRIMARY KEY,
        cursor_id BIGINT NOT NULL DEFAULT 0,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
    )",
    "CREATE TABLE IF NOT EXISTS retention_prepared_archives (
        prepared_key TEXT PRIMARY KEY,
        dataset TEXT NOT NULL,
        month_key TEXT NOT NULL,
        day_key TEXT,
        part_key TEXT,
        file_path TEXT NOT NULL,
        source_ids_json TEXT NOT NULL,
        source_identity_sha256 TEXT NOT NULL,
        publication_kind TEXT,
        state TEXT NOT NULL,
        artifact_sha256 TEXT,
        artifact_bytes BIGINT,
        attempt_count BIGINT NOT NULL DEFAULT 0,
        next_retry_at TEXT,
        last_failure_stage TEXT,
        last_failure_fingerprint TEXT,
        staged_file_path TEXT,
        created_at_epoch_ms BIGINT NOT NULL,
        updated_at_epoch_ms BIGINT NOT NULL,
        quarantined_at TEXT,
        UNIQUE (dataset, file_path)
    )",
    "CREATE TABLE IF NOT EXISTS retention_recovery_cursors (
        scope TEXT PRIMARY KEY,
        cursor TEXT NOT NULL,
        next_retry_at TEXT,
        consecutive_failure_count BIGINT NOT NULL DEFAULT 0,
        last_failure_fingerprint TEXT,
        defer_reason TEXT,
        last_progress_at TEXT,
        last_admission_stage TEXT,
        last_admission_cause TEXT,
        last_settled_pass_at TEXT,
        last_settled_pass_complete BOOLEAN,
        last_settled_pass_inspected_entries BIGINT,
        last_settled_pass_referenced_skipped BIGINT,
        last_settled_pass_quarantined BIGINT,
        last_settled_pass_removed BIGINT,
        last_settled_pass_removed_bytes BIGINT,
        last_nonzero_removal_at TEXT,
        last_nonzero_removal BOOLEAN,
        last_nonzero_removal_bytes BIGINT,
        updated_at_epoch_ms BIGINT NOT NULL
    )",
    "CREATE TABLE IF NOT EXISTS retention_raw_reconciliation (
        raw_path TEXT PRIMARY KEY,
        file_identity_sha256 TEXT NOT NULL,
        byte_size BIGINT NOT NULL,
        quarantined_at TEXT NOT NULL,
        release_pending BOOLEAN NOT NULL DEFAULT FALSE,
        release_pending_at TEXT,
        removal_evidence_persisted BOOLEAN NOT NULL DEFAULT FALSE,
        updated_at_epoch_ms BIGINT NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_archive_raw_reference_changes_raw_active
        ON archive_raw_reference_changes (raw_path, state, action)",
    "CREATE INDEX IF NOT EXISTS idx_proxy_raw_payload_blob_links_path
        ON proxy_raw_payload_blob_links (raw_path)",
    "CREATE OR REPLACE FUNCTION lock_proxy_raw_payload_blob_link_path()
     RETURNS trigger
     LANGUAGE plpgsql
     AS $$
     BEGIN
         IF TG_OP = 'DELETE' THEN
             PERFORM pg_advisory_xact_lock(hashtextextended(OLD.raw_path, 0));
             RETURN OLD;
         END IF;
         IF TG_OP = 'UPDATE' AND OLD.raw_path <> NEW.raw_path THEN
             IF OLD.raw_path < NEW.raw_path THEN
                 PERFORM pg_advisory_xact_lock(hashtextextended(OLD.raw_path, 0));
                 PERFORM pg_advisory_xact_lock(hashtextextended(NEW.raw_path, 0));
             ELSE
                 PERFORM pg_advisory_xact_lock(hashtextextended(NEW.raw_path, 0));
                 PERFORM pg_advisory_xact_lock(hashtextextended(OLD.raw_path, 0));
             END IF;
         ELSE
             PERFORM pg_advisory_xact_lock(hashtextextended(NEW.raw_path, 0));
         END IF;
         RETURN NEW;
     END
     $$",
    "DROP TRIGGER IF EXISTS trg_lock_proxy_raw_payload_blob_link_path
       ON proxy_raw_payload_blob_links",
    "CREATE TRIGGER trg_lock_proxy_raw_payload_blob_link_path
     BEFORE INSERT OR UPDATE OR DELETE ON proxy_raw_payload_blob_links
     FOR EACH ROW EXECUTE FUNCTION lock_proxy_raw_payload_blob_link_path()",
    "CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_file
        ON pool_upstream_node_health_archive (archive_file_path)",
    "CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_archive_occurred_at_binding
        ON pool_upstream_node_health_archive (occurred_at, proxy_binding_key_snapshot)",
    "CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_file
        ON pool_upstream_node_health_hourly_archive (archive_file_path)",
    "CREATE INDEX IF NOT EXISTS idx_pool_upstream_node_health_hourly_archive_batch
        ON pool_upstream_node_health_hourly_archive (archive_batch_id)",
    "CREATE INDEX IF NOT EXISTS idx_archive_account_activity_outbox_batch_state
        ON archive_account_activity_outbox (archive_batch_id, state)",
    "CREATE INDEX IF NOT EXISTS idx_hourly_rollup_archive_replay_file
        ON hourly_rollup_archive_replay (dataset, file_path, archive_sha256)",
];

const ARCHIVE_STORAGE_SCHEMA_UPGRADES: &[&str] = &[
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'completed'",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS writer_version TEXT NOT NULL DEFAULT 'legacy_month_v1'",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS cleanup_source_safe_start_date TEXT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS replacement_staged_path TEXT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS superseded_by BIGINT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS archive_expires_at TEXT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS summary_source_kind TEXT NOT NULL DEFAULT 'unknown'",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS upstream_activity_manifest_refreshed_at TEXT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS historical_rollups_materialized_at TEXT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS coverage_start_epoch BIGINT",
    "ALTER TABLE archive_batches ADD COLUMN IF NOT EXISTS coverage_end_epoch BIGINT",
    "ALTER TABLE archive_prune_ledger ADD COLUMN IF NOT EXISTS file_path TEXT",
    // Legacy rows that still have their manifest can be repaired from the authoritative path.
    // Rows whose metadata was already pruned intentionally remain NULL and fail closed.
    "UPDATE archive_prune_ledger AS ledger
        SET file_path = batches.file_path
       FROM archive_batches AS batches
      WHERE ledger.file_path IS NULL
        AND ledger.metadata_pruned_at_epoch_ms IS NULL
        AND ledger.archive_batch_id = batches.archive_batch_id",
    "ALTER TABLE proxy_raw_payload_blobs ADD COLUMN IF NOT EXISTS file_identity_sha256 TEXT",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveManifest {
    pub(crate) archive_batch_id: i64,
    pub(crate) dataset: String,
    pub(crate) month_key: String,
    pub(crate) day_key: String,
    pub(crate) part: i32,
    pub(crate) file_path: String,
    pub(crate) sha256: String,
    pub(crate) source_identity_sha256: String,
    pub(crate) row_count: i64,
    pub(crate) layout: String,
    pub(crate) codec: String,
    pub(crate) cleanup_state: String,
    pub(crate) summary_source_kind: String,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
    pub(crate) historical_rollups_materialized_at: Option<String>,
}

impl ArchiveManifest {
    pub(crate) fn validate(&self, archive_root: &Path) -> Result<()> {
        if self.archive_batch_id <= 0 {
            bail!("archive manifest requires a positive batch id");
        }
        if self.dataset.trim().is_empty() || self.month_key.trim().is_empty() {
            bail!("archive manifest requires dataset and month key");
        }
        if self.part < 0 || self.row_count < 0 {
            bail!("archive manifest part and row count cannot be negative");
        }
        validate_sha256("archive manifest sha256", &self.sha256)?;
        validate_sha256("archive source identity", &self.source_identity_sha256)?;
        normalize_archive_path(&self.file_path, archive_root)?;
        if !matches!(
            self.layout.as_str(),
            "legacy_month" | "segment_v1" | "snapshot_v2"
        ) {
            bail!("unsupported archive layout: {}", self.layout);
        }
        if self.layout != "legacy_month" && self.day_key.trim().is_empty() {
            bail!("segmented archive manifests require a day key");
        }
        if !matches!(self.codec.as_str(), "gzip" | "zstd" | "none") {
            bail!("unsupported archive codec: {}", self.codec);
        }
        if !matches!(
            self.cleanup_state.as_str(),
            "active" | "retained" | "delete_pending" | "deleted"
        ) {
            bail!("unsupported archive cleanup state: {}", self.cleanup_state);
        }
        if !matches!(
            self.summary_source_kind.as_str(),
            SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE | SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
        ) {
            bail!(
                "new archive manifests require an explicit summary source kind: {}",
                self.summary_source_kind
            );
        }
        if let (Some(start), Some(end)) = (&self.coverage_start_at, &self.coverage_end_at)
            && start > end
        {
            bail!("archive coverage start is after its end");
        }
        if self.dataset == "codex_invocations"
            && self.summary_source_kind == SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
            && (self.coverage_start_at.is_none()
                || self.coverage_end_at.is_none()
                || self.historical_rollups_materialized_at.is_none())
        {
            bail!(
                "authoritative invocation archives require bounded coverage and materialized rollups"
            );
        }
        Ok(())
    }

    pub(crate) fn canonicalized(&self, archive_root: &Path) -> Result<Self> {
        self.validate(archive_root)?;
        let mut canonical = self.clone();
        canonical.file_path = normalize_archive_path(&self.file_path, archive_root)?;
        canonical.validate(archive_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionPreparedArchive {
    pub(crate) prepared_key: String,
    pub(crate) dataset: String,
    pub(crate) month_key: String,
    pub(crate) day_key: Option<String>,
    pub(crate) part_key: Option<String>,
    pub(crate) file_path: String,
    pub(crate) source_ids_json: String,
    pub(crate) source_identity_sha256: String,
    pub(crate) publication_kind: Option<String>,
    pub(crate) state: String,
    pub(crate) artifact_sha256: Option<String>,
    pub(crate) artifact_bytes: Option<i64>,
    pub(crate) attempt_count: i64,
    pub(crate) next_retry_at: Option<String>,
    pub(crate) last_failure_stage: Option<String>,
    pub(crate) last_failure_fingerprint: Option<String>,
    pub(crate) staged_file_path: Option<String>,
    pub(crate) created_at_epoch_ms: i64,
    pub(crate) updated_at_epoch_ms: i64,
    pub(crate) quarantined_at: Option<String>,
}

impl RetentionPreparedArchive {
    pub(crate) fn validate(&self, archive_root: &Path) -> Result<()> {
        if self.prepared_key.trim().is_empty()
            || self.dataset.trim().is_empty()
            || self.month_key.trim().is_empty()
            || self.source_ids_json.trim().is_empty()
            || self.state.trim().is_empty()
        {
            bail!("prepared archive requires stable identity and state");
        }
        let source_ids: Value = serde_json::from_str(&self.source_ids_json)
            .context("prepared archive source ids must be valid JSON")?;
        if !source_ids.is_array() {
            bail!("prepared archive source ids must be a JSON array");
        }
        normalize_archive_path(&self.file_path, archive_root)?;
        if let Some(staged_path) = &self.staged_file_path {
            normalize_archive_path(staged_path, archive_root)?;
        }
        validate_sha256(
            "prepared archive source identity",
            &self.source_identity_sha256,
        )?;
        if let Some(artifact_sha256) = &self.artifact_sha256 {
            validate_sha256("prepared archive artifact", artifact_sha256)?;
        }
        if self.artifact_bytes.unwrap_or_default() < 0
            || self.attempt_count < 0
            || self.created_at_epoch_ms < 0
            || self.updated_at_epoch_ms < self.created_at_epoch_ms
        {
            bail!("prepared archive counters and timestamps are invalid");
        }
        Ok(())
    }

    pub(crate) fn canonicalized(&self, archive_root: &Path) -> Result<Self> {
        self.validate(archive_root)?;
        let mut canonical = self.clone();
        canonical.file_path = normalize_archive_path(&self.file_path, archive_root)?;
        canonical.staged_file_path = self
            .staged_file_path
            .as_deref()
            .map(|path| normalize_archive_path(path, archive_root))
            .transpose()?;
        canonical.validate(archive_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveProof {
    pub(crate) archive_batch_id: i64,
    pub(crate) manifest_sha256: String,
    pub(crate) page_count: i64,
    pub(crate) row_count: i64,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
    pub(crate) semantic_sha256: String,
    pub(crate) verified_at_epoch_ms: i64,
}

impl ArchiveProof {
    pub(crate) fn validate_for(&self, manifest: &ArchiveManifest) -> Result<()> {
        if self.archive_batch_id != manifest.archive_batch_id {
            bail!("archive proof does not belong to the manifest");
        }
        validate_sha256("archive proof manifest sha256", &self.manifest_sha256)?;
        validate_sha256("archive proof semantic sha256", &self.semantic_sha256)?;
        if self.manifest_sha256 != manifest.sha256 {
            bail!("archive proof manifest digest does not match the archive");
        }
        if self.page_count < 0 || self.row_count < 0 || self.verified_at_epoch_ms < 0 {
            bail!("archive proof counters and verification time cannot be negative");
        }
        if self.row_count != manifest.row_count {
            bail!("archive proof row count does not match the manifest");
        }
        if self.row_count > 0 && self.page_count == 0 {
            bail!("a non-empty archive requires at least one proof page");
        }
        if self.coverage_start_at != manifest.coverage_start_at
            || self.coverage_end_at != manifest.coverage_end_at
        {
            bail!("archive proof coverage does not match the manifest");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveRollupMarker {
    pub(crate) archive_batch_id: i64,
    pub(crate) target: String,
    pub(crate) bucket_start_epoch_ms: i64,
    pub(crate) source_identity_sha256: String,
    pub(crate) materialized_rows: i64,
    pub(crate) complete: bool,
}

impl ArchiveRollupMarker {
    pub(crate) fn validate(&self, manifest: &ArchiveManifest) -> Result<()> {
        if self.archive_batch_id != manifest.archive_batch_id {
            bail!("archive rollup marker does not belong to the manifest");
        }
        if self.target.trim().is_empty() {
            bail!("archive rollup marker requires a target");
        }
        if self.bucket_start_epoch_ms < 0 || self.materialized_rows < 0 {
            bail!("archive rollup marker counters cannot be negative");
        }
        validate_sha256(
            "archive rollup source identity",
            &self.source_identity_sha256,
        )?;
        if self.source_identity_sha256 != manifest.source_identity_sha256 {
            bail!("archive rollup source identity does not match the manifest");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PoolUpstreamNodeHealthArchiveRow {
    pub(crate) archived_row_id: i64,
    pub(crate) occurred_at: String,
    pub(crate) proxy_binding_key_snapshot: String,
    pub(crate) is_success: i64,
    pub(crate) latency_ms: Option<f64>,
}

impl PoolUpstreamNodeHealthArchiveRow {
    fn validate(&self) -> Result<()> {
        if self.archived_row_id <= 0
            || self.occurred_at.trim().is_empty()
            || self.proxy_binding_key_snapshot.trim().is_empty()
        {
            bail!("node-health archive row requires stable identity and dimensions");
        }
        if !matches!(self.is_success, 0 | 1) {
            bail!("node-health archive success flag must be zero or one");
        }
        if self
            .latency_ms
            .is_some_and(|latency| !latency.is_finite() || latency < 0.0)
        {
            bail!("node-health archive latency must be finite and non-negative");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PoolUpstreamNodeHealthHourlyArchiveRow {
    pub(crate) proxy_binding_key_snapshot: String,
    pub(crate) bucket_start_epoch: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

impl PoolUpstreamNodeHealthHourlyArchiveRow {
    fn validate(&self) -> Result<()> {
        if self.proxy_binding_key_snapshot.trim().is_empty()
            || self.bucket_start_epoch < 0
            || self.success_count < 0
            || self.failure_count < 0
        {
            bail!("node-health hourly archive row has invalid dimensions or counters");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveReplayMarker {
    pub(crate) target: String,
    pub(crate) dataset: String,
    pub(crate) file_path: String,
    pub(crate) archive_sha256: String,
}

impl ArchiveReplayMarker {
    fn validate(&self, manifest: &ArchiveManifest) -> Result<()> {
        if self.target.trim().is_empty()
            || self.dataset.trim().is_empty()
            || self.file_path.trim().is_empty()
        {
            bail!("archive replay marker requires target, dataset, and file path");
        }
        if self.dataset != manifest.dataset || self.file_path != manifest.file_path {
            bail!("archive replay marker does not match the archive manifest");
        }
        validate_sha256("archive replay marker", &self.archive_sha256)?;
        if self.archive_sha256 != manifest.sha256 {
            bail!("archive replay marker digest does not match the archive");
        }
        Ok(())
    }

    fn canonicalized(&self, manifest: &ArchiveManifest, archive_root: &Path) -> Result<Self> {
        let mut canonical = self.clone();
        canonical.file_path = normalize_archive_path(&self.file_path, archive_root)?;
        canonical.validate(manifest)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawReferenceAction {
    Link,
    Release,
}

impl RawReferenceAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Release => "release",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawOwnershipCounts {
    pub(crate) live_links: i64,
    pub(crate) archive_links: i64,
    pub(crate) distinct_links: i64,
}

impl RawOwnershipCounts {
    pub(crate) fn can_remove_blob(self) -> Result<bool> {
        if self.live_links < 0 || self.archive_links < 0 || self.distinct_links < 0 {
            bail!("Raw ownership counts cannot be negative");
        }
        Ok(self.live_links == 0 && self.archive_links == 0 && self.distinct_links == 0)
    }
}

pub(crate) fn raw_paths_in_lock_order<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<&'a str> {
    paths.into_iter().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawReferenceChange {
    pub(crate) archive_batch_id: i64,
    pub(crate) raw_path: String,
    pub(crate) owner_kind: String,
    pub(crate) owner_id: i64,
    pub(crate) raw_role: String,
    pub(crate) action: RawReferenceAction,
    pub(crate) raw_identity_sha256: Option<String>,
}

impl RawReferenceChange {
    pub(crate) fn validate(&self, manifest: &ArchiveManifest, raw_root: &Path) -> Result<()> {
        if self.archive_batch_id != manifest.archive_batch_id {
            bail!("Raw reference change does not belong to the manifest");
        }
        if self.owner_kind.trim().is_empty() || self.raw_role.trim().is_empty() {
            bail!("Raw reference change requires owner and role");
        }
        if !matches!(self.owner_kind.as_str(), "invocation" | "attempt") {
            bail!("unsupported Raw reference owner kind: {}", self.owner_kind);
        }
        if !matches!(self.raw_role.as_str(), "request" | "response") {
            bail!("unsupported Raw reference role: {}", self.raw_role);
        }
        if self.owner_id <= 0 {
            bail!("Raw reference owner id must be positive");
        }
        normalize_raw_path(&self.raw_path, raw_root)?;
        let identity = self
            .raw_identity_sha256
            .as_deref()
            .context("Raw reference change requires a file identity")?;
        validate_sha256("Raw reference identity", identity)?;
        Ok(())
    }

    fn canonicalized(&self, manifest: &ArchiveManifest, raw_root: &Path) -> Result<Self> {
        self.validate(manifest, raw_root)?;
        let mut canonical = self.clone();
        canonical.raw_path = normalize_raw_path(&self.raw_path, raw_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawReferenceConversion {
    pub(crate) legacy_path: String,
    pub(crate) canonical_path: String,
    pub(crate) aliases: Vec<String>,
}

impl RawReferenceConversion {
    /// Convert an existing Raw path without touching the filesystem. The aliases preserve the
    /// old spelling for readers during a gradual conversion and are deterministic for retries.
    pub(crate) fn from_legacy_path(legacy_path: &str, raw_root: &Path) -> Result<Self> {
        let canonical_path = normalize_raw_path(legacy_path, raw_root)?;
        let mut aliases = BTreeSet::new();
        aliases.insert(legacy_path.trim().to_owned());
        aliases.insert(canonical_path.clone());
        if let Some(root_name) = raw_root.file_name().and_then(|name| name.to_str()) {
            let root_prefixed = Path::new(root_name).join(&canonical_path);
            if let Some(value) = root_prefixed.to_str() {
                aliases.insert(value.to_owned());
            }
        }
        Ok(Self {
            legacy_path: legacy_path.trim().to_owned(),
            canonical_path,
            aliases: aliases.into_iter().collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoricalAccountActivityOutboxItem {
    pub(crate) archive_batch_id: i64,
    pub(crate) account_id: i64,
    pub(crate) last_activity_at: String,
    pub(crate) source_revision: i64,
    pub(crate) control_generation: i64,
}

impl HistoricalAccountActivityOutboxItem {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.archive_batch_id <= 0 || self.account_id <= 0 {
            bail!("historical account activity requires positive identities");
        }
        if self.last_activity_at.trim().is_empty() {
            bail!("historical account activity requires a timestamp");
        }
        if self.source_revision < 0 || self.control_generation <= 0 {
            bail!("historical account activity generations cannot be negative");
        }
        Ok(())
    }

    pub(crate) fn idempotency_key(&self) -> Result<String> {
        self.validate()?;
        Ok(sha256_hex(
            format!(
                "archive-account-activity:{}:{}:{}:{}:{}",
                self.archive_batch_id,
                self.account_id,
                self.last_activity_at,
                self.source_revision,
                self.control_generation
            )
            .as_bytes(),
        ))
    }
}

/// A control database applies an observation only while its current generation is no newer
/// than the archive observation. Equal generations are safe to retry because the timestamp is
/// compared as a monotonic max by the control module.
pub(crate) fn can_apply_historical_activity(
    item: &HistoricalAccountActivityOutboxItem,
    current_control_generation: i64,
    current_last_activity_at: Option<&str>,
) -> Result<bool> {
    item.validate()?;
    if current_control_generation <= 0 {
        bail!("current control generation must be positive");
    }
    if current_control_generation > item.control_generation {
        return Ok(false);
    }
    Ok(current_last_activity_at
        .map(|current| item.last_activity_at.as_str() > current)
        .unwrap_or(true))
}

pub(crate) fn deduplicate_activity_manifest(
    archive_batch_id: i64,
    values: &[HistoricalAccountActivityOutboxItem],
) -> Result<BTreeMap<i64, HistoricalAccountActivityOutboxItem>> {
    if archive_batch_id <= 0 {
        bail!("archive batch id must be positive");
    }
    let mut deduplicated = BTreeMap::<i64, HistoricalAccountActivityOutboxItem>::new();
    for item in values {
        item.validate()?;
        if item.archive_batch_id != archive_batch_id {
            bail!("archived account activity does not belong to the batch");
        }
        deduplicated
            .entry(item.account_id)
            .and_modify(|current| {
                let current_key = (
                    current.last_activity_at.as_str(),
                    current.source_revision,
                    current.control_generation,
                );
                let item_key = (
                    item.last_activity_at.as_str(),
                    item.source_revision,
                    item.control_generation,
                );
                if current_key < item_key {
                    *current = item.clone();
                }
            })
            .or_insert_with(|| item.clone());
    }
    Ok(deduplicated)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawReferenceCheck {
    pub(crate) raw_path: String,
    /// Distinct active owner tuples from the live and archive ledgers at preparation time.
    pub(crate) active_reference_count: i64,
    pub(crate) file_identity_sha256: String,
    pub(crate) checked_at_epoch_ms: i64,
}

impl RawReferenceCheck {
    pub(crate) fn validate(&self, raw_root: &Path) -> Result<()> {
        normalize_raw_path(&self.raw_path, raw_root)?;
        if self.active_reference_count < 0 || self.checked_at_epoch_ms < 0 {
            bail!("Raw reference check counters cannot be negative");
        }
        validate_sha256("Raw reference file identity", &self.file_identity_sha256)
    }

    fn canonicalized(&self, raw_root: &Path) -> Result<Self> {
        self.validate(raw_root)?;
        let mut canonical = self.clone();
        canonical.raw_path = normalize_raw_path(&self.raw_path, raw_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionRecoveryCursor {
    pub(crate) scope: String,
    pub(crate) cursor: String,
    pub(crate) next_retry_at: Option<String>,
    pub(crate) consecutive_failure_count: i64,
    pub(crate) last_failure_fingerprint: Option<String>,
    pub(crate) defer_reason: Option<String>,
    pub(crate) last_progress_at: Option<String>,
    pub(crate) last_admission_stage: Option<String>,
    pub(crate) last_admission_cause: Option<String>,
    pub(crate) last_settled_pass_at: Option<String>,
    pub(crate) last_settled_pass_complete: Option<bool>,
    pub(crate) last_settled_pass_inspected_entries: Option<i64>,
    pub(crate) last_settled_pass_referenced_skipped: Option<i64>,
    pub(crate) last_settled_pass_quarantined: Option<i64>,
    pub(crate) last_settled_pass_removed: Option<i64>,
    pub(crate) last_settled_pass_removed_bytes: Option<i64>,
    pub(crate) last_nonzero_removal_at: Option<String>,
    pub(crate) last_nonzero_removal: Option<bool>,
    pub(crate) last_nonzero_removal_bytes: Option<i64>,
    pub(crate) updated_at_epoch_ms: i64,
}

impl RetentionRecoveryCursor {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.scope.trim().is_empty() {
            bail!("retention recovery cursor requires a scope");
        }
        if self.consecutive_failure_count < 0 || self.updated_at_epoch_ms < 0 {
            bail!("retention recovery cursor counters cannot be negative");
        }
        for value in [
            self.last_settled_pass_inspected_entries,
            self.last_settled_pass_referenced_skipped,
            self.last_settled_pass_quarantined,
            self.last_settled_pass_removed,
            self.last_settled_pass_removed_bytes,
            self.last_nonzero_removal_bytes,
        ]
        .into_iter()
        .flatten()
        {
            if value < 0 {
                bail!("retention recovery cursor pass counters cannot be negative");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetentionRawReconciliation {
    pub(crate) raw_path: String,
    pub(crate) file_identity_sha256: String,
    pub(crate) byte_size: i64,
    pub(crate) quarantined_at: String,
    pub(crate) release_pending: bool,
    pub(crate) release_pending_at: Option<String>,
    pub(crate) removal_evidence_persisted: bool,
    pub(crate) updated_at_epoch_ms: i64,
}

impl RetentionRawReconciliation {
    pub(crate) fn validate(&self, raw_root: &Path) -> Result<()> {
        normalize_raw_path(&self.raw_path, raw_root)?;
        validate_sha256("retention Raw identity", &self.file_identity_sha256)?;
        if self.byte_size < 0 || self.updated_at_epoch_ms < 0 {
            bail!("retention Raw reconciliation counters cannot be negative");
        }
        if self.quarantined_at.trim().is_empty() {
            bail!("retention Raw reconciliation requires quarantine time");
        }
        Ok(())
    }

    fn canonicalized(&self, raw_root: &Path) -> Result<Self> {
        self.validate(raw_root)?;
        let mut canonical = self.clone();
        canonical.raw_path = normalize_raw_path(&self.raw_path, raw_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchivePrunePlan {
    pub(crate) manifest: ArchiveManifest,
    pub(crate) proof: Option<ArchiveProof>,
    pub(crate) rollups: Vec<ArchiveRollupMarker>,
    pub(crate) raw_reference_checks: Vec<RawReferenceCheck>,
    pub(crate) pending_activity_outbox: i64,
    pub(crate) file_identity_sha256: String,
}

impl ArchivePrunePlan {
    pub(crate) fn validate(&self, archive_root: &Path, raw_root: &Path) -> Result<()> {
        self.manifest.validate(archive_root)?;
        if self.manifest.summary_source_kind == SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE {
            self.proof
                .as_ref()
                .context("authoritative archive pruning requires a publication proof")?
                .validate_for(&self.manifest)?;
        } else if self.proof.is_some() {
            bail!("live-mirror archives cannot carry a Summary publication proof");
        }
        if self.manifest.cleanup_state != "delete_pending" {
            bail!("archive must be delete_pending before metadata pruning");
        }
        if self.pending_activity_outbox != 0 {
            bail!("archive has pending historical account activity outbox items");
        }
        validate_sha256("archive prune file identity", &self.file_identity_sha256)?;
        if self.file_identity_sha256 != self.manifest.sha256 {
            bail!("archive prune identity does not match the manifest");
        }
        let mut checked_raw_paths = BTreeSet::new();
        for check in &self.raw_reference_checks {
            check.validate(raw_root)?;
            if check.active_reference_count != 0 {
                bail!("archive still has active Raw references");
            }
            if !checked_raw_paths.insert(check.raw_path.clone()) {
                bail!("archive prune repeats a Raw reference check");
            }
        }
        let requires_rollups =
            self.manifest.summary_source_kind == SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
        if requires_rollups && self.rollups.is_empty() {
            bail!("authoritative archive pruning requires rollup markers");
        }
        if !requires_rollups && !self.rollups.is_empty() {
            bail!("live-mirror archives cannot carry Summary rollup markers");
        }
        for marker in &self.rollups {
            marker.validate(&self.manifest)?;
            if self.manifest.dataset == "codex_invocations" && !marker.complete {
                bail!("authoritative invocation archives require complete rollup markers");
            }
            if requires_rollups && !marker.complete {
                bail!("archive has an incomplete rollup marker");
            }
        }
        let mut rollup_keys = BTreeSet::new();
        for marker in &self.rollups {
            if !rollup_keys.insert(format!(
                "{}:{}",
                marker.target, marker.bucket_start_epoch_ms
            )) {
                bail!("archive prune repeats a rollup marker");
            }
        }
        Ok(())
    }

    pub(crate) fn canonicalized(&self, archive_root: &Path, raw_root: &Path) -> Result<Self> {
        self.validate(archive_root, raw_root)?;
        let mut canonical = self.clone();
        canonical.manifest = self.manifest.canonicalized(archive_root)?;
        canonical.raw_reference_checks = self
            .raw_reference_checks
            .iter()
            .map(|check| check.canonicalized(raw_root))
            .collect::<Result<Vec<_>>>()?;
        canonical.validate(archive_root, raw_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ArchivePublishOperation {
    pub(crate) manifest: ArchiveManifest,
    pub(crate) proof: Option<ArchiveProof>,
    pub(crate) rollups: Vec<ArchiveRollupMarker>,
    pub(crate) replay_markers: Vec<ArchiveReplayMarker>,
    pub(crate) node_health_rows: Vec<PoolUpstreamNodeHealthArchiveRow>,
    pub(crate) node_health_hourly_rows: Vec<PoolUpstreamNodeHealthHourlyArchiveRow>,
    pub(crate) raw_reference_changes: Vec<RawReferenceChange>,
    pub(crate) historical_activity: Vec<HistoricalAccountActivityOutboxItem>,
    pub(crate) retention_prepared_archive: Option<RetentionPreparedArchive>,
    pub(crate) retention_recovery_cursor: Option<RetentionRecoveryCursor>,
    pub(crate) retention_raw_reconciliation: Vec<RetentionRawReconciliation>,
}

impl ArchivePublishOperation {
    pub(crate) fn validate(&self, archive_root: &Path, raw_root: &Path) -> Result<()> {
        self.manifest.validate(archive_root)?;
        let authoritative =
            self.manifest.summary_source_kind == SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE;
        if authoritative {
            self.proof
                .as_ref()
                .context("authoritative archives require a publication proof")?
                .validate_for(&self.manifest)?;
        } else if self.proof.is_some() {
            bail!("live-mirror archives cannot carry a Summary publication proof");
        }
        if authoritative && self.rollups.is_empty() {
            bail!("authoritative archive publication requires a rollup marker");
        }
        if !authoritative && !self.rollups.is_empty() {
            bail!("live-mirror archives cannot carry Summary rollup markers");
        }
        for marker in &self.rollups {
            marker.validate(&self.manifest)?;
            if self.manifest.dataset == "codex_invocations" && !marker.complete {
                bail!("authoritative invocation archives require complete rollup markers");
            }
        }
        let mut rollup_keys = BTreeSet::new();
        for marker in &self.rollups {
            if !rollup_keys.insert(format!(
                "{}:{}",
                marker.target, marker.bucket_start_epoch_ms
            )) {
                bail!("archive publication repeats a rollup marker");
            }
        }
        if authoritative && self.replay_markers.is_empty() {
            bail!("authoritative archive publication requires a replay marker");
        }
        if !authoritative && !self.replay_markers.is_empty() {
            bail!("live-mirror archives cannot carry Summary replay markers");
        }
        let mut replay_keys = BTreeSet::new();
        for marker in &self.replay_markers {
            marker.validate(&self.manifest)?;
            if !replay_keys.insert(format!(
                "{}:{}:{}",
                marker.target, marker.dataset, marker.file_path
            )) {
                bail!("archive publication repeats a replay marker");
            }
        }
        let replay_targets = self
            .replay_markers
            .iter()
            .map(|marker| marker.target.as_str())
            .collect::<BTreeSet<_>>();
        if self.manifest.dataset == "codex_invocations"
            && authoritative
            && INVOCATION_SUMMARY_REPLAY_TARGETS
                .iter()
                .any(|target| !replay_targets.contains(target))
        {
            bail!("authoritative invocation archives require all Summary replay markers");
        }
        if (!self.node_health_rows.is_empty() || !self.node_health_hourly_rows.is_empty())
            && self.manifest.dataset != "pool_upstream_request_attempts"
        {
            bail!("node-health archive rows require upstream attempt archives");
        }
        let mut node_health_keys = BTreeSet::new();
        for row in &self.node_health_rows {
            row.validate()?;
            if !node_health_keys.insert(row.archived_row_id) {
                bail!("archive publication repeats a node-health archive row");
            }
        }
        let mut node_health_hourly_keys = BTreeSet::new();
        for row in &self.node_health_hourly_rows {
            row.validate()?;
            if !node_health_hourly_keys.insert(format!(
                "{}:{}",
                row.proxy_binding_key_snapshot, row.bucket_start_epoch
            )) {
                bail!("archive publication repeats a node-health hourly row");
            }
        }
        if !self.node_health_rows.is_empty()
            && !replay_targets.contains("pool_upstream_node_health_archive")
        {
            bail!("node-health archive rows require a replay marker");
        }
        if !self.node_health_hourly_rows.is_empty()
            && !replay_targets.contains("pool_upstream_node_health_hourly_archive")
        {
            bail!("node-health hourly rows require a replay marker");
        }
        let mut raw_change_keys = BTreeSet::new();
        for change in &self.raw_reference_changes {
            change.validate(&self.manifest, raw_root)?;
            if !raw_change_keys.insert(format!(
                "{}:{}:{}:{}",
                change.raw_path, change.owner_kind, change.owner_id, change.raw_role
            )) {
                bail!("archive publication repeats a Raw reference change");
            }
        }
        let mut activity_keys = BTreeSet::new();
        for item in &self.historical_activity {
            item.validate()?;
            if item.archive_batch_id != self.manifest.archive_batch_id {
                bail!("historical activity item does not belong to the manifest");
            }
            if !activity_keys.insert(item.idempotency_key()?) {
                bail!("archive publication repeats historical account activity");
            }
        }
        if let Some(prepared_archive) = &self.retention_prepared_archive {
            prepared_archive.validate(archive_root)?;
            if prepared_archive.dataset != self.manifest.dataset
                || prepared_archive.file_path != self.manifest.file_path
            {
                bail!("prepared archive does not match the archive manifest");
            }
        }
        if let Some(recovery_cursor) = &self.retention_recovery_cursor {
            recovery_cursor.validate()?;
        }
        let mut reconciliation_paths = BTreeSet::new();
        for reconciliation in &self.retention_raw_reconciliation {
            reconciliation.validate(raw_root)?;
            if !reconciliation_paths.insert(reconciliation.raw_path.clone()) {
                bail!("archive publication repeats a Raw reconciliation path");
            }
        }
        Ok(())
    }

    pub(crate) fn canonicalized(&self, archive_root: &Path, raw_root: &Path) -> Result<Self> {
        let mut canonical = self.clone();
        canonical.manifest = self.manifest.canonicalized(archive_root)?;
        canonical.replay_markers = self
            .replay_markers
            .iter()
            .map(|marker| marker.canonicalized(&canonical.manifest, archive_root))
            .collect::<Result<Vec<_>>>()?;
        canonical.raw_reference_changes = self
            .raw_reference_changes
            .iter()
            .map(|change| change.canonicalized(&canonical.manifest, raw_root))
            .collect::<Result<Vec<_>>>()?;
        canonical.retention_prepared_archive = self
            .retention_prepared_archive
            .as_ref()
            .map(|prepared| prepared.canonicalized(archive_root))
            .transpose()?;
        canonical.retention_raw_reconciliation = self
            .retention_raw_reconciliation
            .iter()
            .map(|reconciliation| reconciliation.canonicalized(raw_root))
            .collect::<Result<Vec<_>>>()?;
        canonical.validate(archive_root, raw_root)?;
        Ok(canonical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArchivePublishReceipt {
    pub(crate) archive_batch_id: i64,
    pub(crate) activity_items_written: usize,
    pub(crate) raw_reference_changes_written: usize,
}

#[derive(Clone)]
pub(crate) struct ArchiveStorageAdapter {
    pool: PgPool,
    archive_root: PathBuf,
    raw_root: PathBuf,
}

pub(crate) async fn ensure_archive_storage_schema(pool: &PgPool) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "SELECT pg_advisory_xact_lock(
                    hashtextextended('codex-vibe-monitor:archive-storage-schema', 0)
                )",
    )
    .execute(&mut *transaction)
    .await
    .context("lock archive storage schema initialization")?;
    for statement in ARCHIVE_STORAGE_SCHEMA {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .with_context(|| "create archive storage table")?;
    }
    for statement in ARCHIVE_STORAGE_SCHEMA_UPGRADES {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .with_context(|| "upgrade archive storage table")?;
    }
    transaction
        .commit()
        .await
        .context("commit archive storage schema initialization")?;
    Ok(())
}

impl ArchiveStorageAdapter {
    pub(crate) fn new(pool: PgPool, archive_root: PathBuf, raw_root: PathBuf) -> Self {
        Self {
            pool,
            archive_root,
            raw_root,
        }
    }

    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        ensure_archive_storage_schema(&self.pool).await
    }

    pub(crate) async fn read_retention_prepared_archive(
        &self,
        prepared_key: &str,
    ) -> Result<Option<RetentionPreparedArchive>> {
        let row = sqlx::query(
            "SELECT prepared_key, dataset, month_key, day_key, part_key, file_path,
                    source_ids_json, source_identity_sha256, publication_kind, state,
                    artifact_sha256, artifact_bytes, attempt_count, next_retry_at,
                    last_failure_stage, last_failure_fingerprint, staged_file_path,
                    created_at_epoch_ms, updated_at_epoch_ms, quarantined_at
               FROM retention_prepared_archives
              WHERE prepared_key = $1",
        )
        .bind(prepared_key)
        .fetch_optional(&self.pool)
        .await
        .context("read retention prepared archive")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value = RetentionPreparedArchive {
            prepared_key: row.try_get("prepared_key")?,
            dataset: row.try_get("dataset")?,
            month_key: row.try_get("month_key")?,
            day_key: row.try_get("day_key")?,
            part_key: row.try_get("part_key")?,
            file_path: row.try_get("file_path")?,
            source_ids_json: row.try_get("source_ids_json")?,
            source_identity_sha256: row.try_get("source_identity_sha256")?,
            publication_kind: row.try_get("publication_kind")?,
            state: row.try_get("state")?,
            artifact_sha256: row.try_get("artifact_sha256")?,
            artifact_bytes: row.try_get("artifact_bytes")?,
            attempt_count: row.try_get("attempt_count")?,
            next_retry_at: row.try_get("next_retry_at")?,
            last_failure_stage: row.try_get("last_failure_stage")?,
            last_failure_fingerprint: row.try_get("last_failure_fingerprint")?,
            staged_file_path: row.try_get("staged_file_path")?,
            created_at_epoch_ms: row.try_get("created_at_epoch_ms")?,
            updated_at_epoch_ms: row.try_get("updated_at_epoch_ms")?,
            quarantined_at: row.try_get("quarantined_at")?,
        };
        Ok(Some(value.canonicalized(&self.archive_root)?))
    }

    pub(crate) async fn read_retention_recovery_cursor(
        &self,
        scope: &str,
    ) -> Result<Option<RetentionRecoveryCursor>> {
        let row = sqlx::query(
            "SELECT scope, cursor, next_retry_at, consecutive_failure_count,
                    last_failure_fingerprint, defer_reason, last_progress_at,
                    last_admission_stage, last_admission_cause, last_settled_pass_at,
                    last_settled_pass_complete, last_settled_pass_inspected_entries,
                    last_settled_pass_referenced_skipped, last_settled_pass_quarantined,
                    last_settled_pass_removed, last_settled_pass_removed_bytes,
                    last_nonzero_removal_at, last_nonzero_removal,
                    last_nonzero_removal_bytes, updated_at_epoch_ms
               FROM retention_recovery_cursors
              WHERE scope = $1",
        )
        .bind(scope)
        .fetch_optional(&self.pool)
        .await
        .context("read retention recovery cursor")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value = RetentionRecoveryCursor {
            scope: row.try_get("scope")?,
            cursor: row.try_get("cursor")?,
            next_retry_at: row.try_get("next_retry_at")?,
            consecutive_failure_count: row.try_get("consecutive_failure_count")?,
            last_failure_fingerprint: row.try_get("last_failure_fingerprint")?,
            defer_reason: row.try_get("defer_reason")?,
            last_progress_at: row.try_get("last_progress_at")?,
            last_admission_stage: row.try_get("last_admission_stage")?,
            last_admission_cause: row.try_get("last_admission_cause")?,
            last_settled_pass_at: row.try_get("last_settled_pass_at")?,
            last_settled_pass_complete: row.try_get("last_settled_pass_complete")?,
            last_settled_pass_inspected_entries: row
                .try_get("last_settled_pass_inspected_entries")?,
            last_settled_pass_referenced_skipped: row
                .try_get("last_settled_pass_referenced_skipped")?,
            last_settled_pass_quarantined: row.try_get("last_settled_pass_quarantined")?,
            last_settled_pass_removed: row.try_get("last_settled_pass_removed")?,
            last_settled_pass_removed_bytes: row.try_get("last_settled_pass_removed_bytes")?,
            last_nonzero_removal_at: row.try_get("last_nonzero_removal_at")?,
            last_nonzero_removal: row.try_get("last_nonzero_removal")?,
            last_nonzero_removal_bytes: row.try_get("last_nonzero_removal_bytes")?,
            updated_at_epoch_ms: row.try_get("updated_at_epoch_ms")?,
        };
        value.validate()?;
        Ok(Some(value))
    }

    pub(crate) async fn read_retention_raw_reconciliation(
        &self,
        raw_path: &str,
    ) -> Result<Option<RetentionRawReconciliation>> {
        let row = sqlx::query(
            "SELECT raw_path, file_identity_sha256, byte_size, quarantined_at,
                    release_pending, release_pending_at, removal_evidence_persisted,
                    updated_at_epoch_ms
               FROM retention_raw_reconciliation
              WHERE raw_path = $1",
        )
        .bind(raw_path)
        .fetch_optional(&self.pool)
        .await
        .context("read retention Raw reconciliation")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value = RetentionRawReconciliation {
            raw_path: row.try_get("raw_path")?,
            file_identity_sha256: row.try_get("file_identity_sha256")?,
            byte_size: row.try_get("byte_size")?,
            quarantined_at: row.try_get("quarantined_at")?,
            release_pending: row.try_get("release_pending")?,
            release_pending_at: row.try_get("release_pending_at")?,
            removal_evidence_persisted: row.try_get("removal_evidence_persisted")?,
            updated_at_epoch_ms: row.try_get("updated_at_epoch_ms")?,
        };
        value.validate(&self.raw_root)?;
        Ok(Some(value))
    }

    /// Publish all archive metadata in one transaction. The SQL writes metadata and outbox
    /// rows only; no file operation or account-routing notification is performed here.
    pub(crate) async fn publish(
        &self,
        operation: &ArchivePublishOperation,
    ) -> Result<ArchivePublishReceipt> {
        let operation = operation.canonicalized(&self.archive_root, &self.raw_root)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(
                        hashtextextended('codex-vibe-monitor:archive-storage-schema', 0)
                    )",
        )
        .execute(&mut *transaction)
        .await
        .context("lock archive storage schema during publication")?;

        sqlx::query(
            "INSERT INTO archive_batches
                (archive_batch_id, dataset, month_key, day_key, part, file_path, sha256,
                 source_identity_sha256, row_count, layout, codec, cleanup_state,
                 coverage_start_at, coverage_end_at, historical_rollups_materialized_at,
                 status, writer_version, summary_source_kind)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                     'completed', $16, $17)
             ON CONFLICT (archive_batch_id) DO NOTHING",
        )
        .bind(operation.manifest.archive_batch_id)
        .bind(&operation.manifest.dataset)
        .bind(&operation.manifest.month_key)
        .bind(&operation.manifest.day_key)
        .bind(operation.manifest.part)
        .bind(&operation.manifest.file_path)
        .bind(&operation.manifest.sha256)
        .bind(&operation.manifest.source_identity_sha256)
        .bind(operation.manifest.row_count)
        .bind(&operation.manifest.layout)
        .bind(&operation.manifest.codec)
        .bind(&operation.manifest.cleanup_state)
        .bind(&operation.manifest.coverage_start_at)
        .bind(&operation.manifest.coverage_end_at)
        .bind(&operation.manifest.historical_rollups_materialized_at)
        .bind(writer_version_for_layout(&operation.manifest.layout))
        .bind(&operation.manifest.summary_source_kind)
        .execute(&mut *transaction)
        .await
        .context("insert archive manifest")?;

        let existing = sqlx::query(
            "SELECT dataset, month_key, day_key, part, file_path, sha256,
                     source_identity_sha256, row_count, status, layout, codec,
                     writer_version, cleanup_state, coverage_start_at, coverage_end_at,
                     summary_source_kind, historical_rollups_materialized_at
               FROM archive_batches WHERE archive_batch_id = $1",
        )
        .bind(operation.manifest.archive_batch_id)
        .fetch_one(&mut *transaction)
        .await
        .context("read archive manifest after insert")?;
        let existing_sha: String = existing.try_get("sha256")?;
        let existing_source_sha: String = existing.try_get("source_identity_sha256")?;
        let existing_dataset: String = existing.try_get("dataset")?;
        let existing_month: String = existing.try_get("month_key")?;
        let existing_day: String = existing
            .try_get::<Option<String>, _>("day_key")?
            .unwrap_or_default();
        let existing_part: i32 = existing.try_get("part")?;
        let existing_path: String = existing.try_get("file_path")?;
        let existing_row_count: i64 = existing.try_get("row_count")?;
        let existing_status: String = existing.try_get("status")?;
        let existing_layout: String = existing.try_get("layout")?;
        let existing_codec: String = existing.try_get("codec")?;
        let existing_writer_version: String = existing.try_get("writer_version")?;
        let existing_cleanup_state: String = existing.try_get("cleanup_state")?;
        let existing_coverage_start: Option<String> = existing.try_get("coverage_start_at")?;
        let existing_coverage_end: Option<String> = existing.try_get("coverage_end_at")?;
        let existing_summary_source_kind: String = existing.try_get("summary_source_kind")?;
        let existing_historical_rollups_materialized_at: Option<String> =
            existing.try_get("historical_rollups_materialized_at")?;
        if existing_dataset != operation.manifest.dataset
            || existing_month != operation.manifest.month_key
            || existing_day != operation.manifest.day_key
            || existing_part != operation.manifest.part
            || existing_path != operation.manifest.file_path
            || existing_sha != operation.manifest.sha256
            || existing_source_sha != operation.manifest.source_identity_sha256
            || existing_row_count != operation.manifest.row_count
            || existing_status != ARCHIVE_STATUS_COMPLETED
            || existing_layout != operation.manifest.layout
            || existing_codec != operation.manifest.codec
            || existing_writer_version != writer_version_for_layout(&operation.manifest.layout)
            || existing_cleanup_state != operation.manifest.cleanup_state
            || existing_coverage_start != operation.manifest.coverage_start_at
            || existing_coverage_end != operation.manifest.coverage_end_at
            || existing_summary_source_kind != operation.manifest.summary_source_kind
            || existing_historical_rollups_materialized_at
                != operation.manifest.historical_rollups_materialized_at
        {
            bail!("archive batch metadata conflicts with existing metadata");
        }

        if let Some(prepared_archive) = &operation.retention_prepared_archive {
            if prepared_archive.state == "quarantined" {
                bail!("quarantined prepared archives cannot be published");
            }
            sqlx::query(
                "INSERT INTO retention_prepared_archives
                    (prepared_key, dataset, month_key, day_key, part_key, file_path,
                     source_ids_json, source_identity_sha256, publication_kind, state,
                     artifact_sha256, artifact_bytes, attempt_count, next_retry_at,
                     last_failure_stage, last_failure_fingerprint, staged_file_path,
                     created_at_epoch_ms, updated_at_epoch_ms, quarantined_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                         $15, $16, $17, $18, $19, $20)
                 ON CONFLICT (prepared_key) DO UPDATE SET
                     dataset = EXCLUDED.dataset,
                     month_key = EXCLUDED.month_key,
                     day_key = EXCLUDED.day_key,
                     part_key = EXCLUDED.part_key,
                     file_path = EXCLUDED.file_path,
                     source_ids_json = EXCLUDED.source_ids_json,
                     source_identity_sha256 = EXCLUDED.source_identity_sha256,
                     publication_kind = EXCLUDED.publication_kind,
                     state = EXCLUDED.state,
                     artifact_sha256 = EXCLUDED.artifact_sha256,
                     artifact_bytes = EXCLUDED.artifact_bytes,
                     attempt_count = EXCLUDED.attempt_count,
                     next_retry_at = EXCLUDED.next_retry_at,
                     last_failure_stage = EXCLUDED.last_failure_stage,
                     last_failure_fingerprint = EXCLUDED.last_failure_fingerprint,
                     staged_file_path = EXCLUDED.staged_file_path,
                     created_at_epoch_ms = EXCLUDED.created_at_epoch_ms,
                     updated_at_epoch_ms = EXCLUDED.updated_at_epoch_ms,
                     quarantined_at = EXCLUDED.quarantined_at
                 WHERE retention_prepared_archives.updated_at_epoch_ms
                       <= EXCLUDED.updated_at_epoch_ms
                   AND retention_prepared_archives.file_path = EXCLUDED.file_path
                   AND retention_prepared_archives.source_identity_sha256
                       = EXCLUDED.source_identity_sha256
                   AND retention_prepared_archives.state <> 'quarantined'",
            )
            .bind(&prepared_archive.prepared_key)
            .bind(&prepared_archive.dataset)
            .bind(&prepared_archive.month_key)
            .bind(&prepared_archive.day_key)
            .bind(&prepared_archive.part_key)
            .bind(&prepared_archive.file_path)
            .bind(&prepared_archive.source_ids_json)
            .bind(&prepared_archive.source_identity_sha256)
            .bind(&prepared_archive.publication_kind)
            .bind(&prepared_archive.state)
            .bind(&prepared_archive.artifact_sha256)
            .bind(prepared_archive.artifact_bytes)
            .bind(prepared_archive.attempt_count)
            .bind(&prepared_archive.next_retry_at)
            .bind(&prepared_archive.last_failure_stage)
            .bind(&prepared_archive.last_failure_fingerprint)
            .bind(&prepared_archive.staged_file_path)
            .bind(prepared_archive.created_at_epoch_ms)
            .bind(prepared_archive.updated_at_epoch_ms)
            .bind(&prepared_archive.quarantined_at)
            .execute(&mut *transaction)
            .await
            .context("upsert retention prepared archive")?;
            let stored = sqlx::query(
                "SELECT file_path, source_identity_sha256, state
                   FROM retention_prepared_archives WHERE prepared_key = $1",
            )
            .bind(&prepared_archive.prepared_key)
            .fetch_one(&mut *transaction)
            .await
            .context("read retention prepared archive after upsert")?;
            let stored_path: String = stored.try_get("file_path")?;
            let stored_source_identity: String = stored.try_get("source_identity_sha256")?;
            let stored_state: String = stored.try_get("state")?;
            if stored_path != prepared_archive.file_path
                || stored_source_identity != prepared_archive.source_identity_sha256
            {
                bail!("retention prepared archive identity conflicts with existing metadata");
            }
            if stored_state == "quarantined" {
                bail!("quarantined prepared archives cannot be published");
            }
        }

        if let Some(recovery_cursor) = &operation.retention_recovery_cursor {
            sqlx::query(
                "INSERT INTO retention_recovery_cursors
                    (scope, cursor, next_retry_at, consecutive_failure_count,
                     last_failure_fingerprint, defer_reason, last_progress_at,
                     last_admission_stage, last_admission_cause, last_settled_pass_at,
                     last_settled_pass_complete, last_settled_pass_inspected_entries,
                     last_settled_pass_referenced_skipped, last_settled_pass_quarantined,
                     last_settled_pass_removed, last_settled_pass_removed_bytes,
                     last_nonzero_removal_at, last_nonzero_removal,
                     last_nonzero_removal_bytes, updated_at_epoch_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                         $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
                 ON CONFLICT (scope) DO UPDATE SET
                     cursor = EXCLUDED.cursor,
                     next_retry_at = EXCLUDED.next_retry_at,
                     consecutive_failure_count = EXCLUDED.consecutive_failure_count,
                     last_failure_fingerprint = EXCLUDED.last_failure_fingerprint,
                     defer_reason = EXCLUDED.defer_reason,
                     last_progress_at = EXCLUDED.last_progress_at,
                     last_admission_stage = EXCLUDED.last_admission_stage,
                     last_admission_cause = EXCLUDED.last_admission_cause,
                     last_settled_pass_at = EXCLUDED.last_settled_pass_at,
                     last_settled_pass_complete = EXCLUDED.last_settled_pass_complete,
                     last_settled_pass_inspected_entries = EXCLUDED.last_settled_pass_inspected_entries,
                     last_settled_pass_referenced_skipped = EXCLUDED.last_settled_pass_referenced_skipped,
                     last_settled_pass_quarantined = EXCLUDED.last_settled_pass_quarantined,
                     last_settled_pass_removed = EXCLUDED.last_settled_pass_removed,
                     last_settled_pass_removed_bytes = EXCLUDED.last_settled_pass_removed_bytes,
                     last_nonzero_removal_at = EXCLUDED.last_nonzero_removal_at,
                     last_nonzero_removal = EXCLUDED.last_nonzero_removal,
                     last_nonzero_removal_bytes = EXCLUDED.last_nonzero_removal_bytes,
                     updated_at_epoch_ms = EXCLUDED.updated_at_epoch_ms
                 WHERE retention_recovery_cursors.updated_at_epoch_ms
                       <= EXCLUDED.updated_at_epoch_ms",
            )
            .bind(&recovery_cursor.scope)
            .bind(&recovery_cursor.cursor)
            .bind(&recovery_cursor.next_retry_at)
            .bind(recovery_cursor.consecutive_failure_count)
            .bind(&recovery_cursor.last_failure_fingerprint)
            .bind(&recovery_cursor.defer_reason)
            .bind(&recovery_cursor.last_progress_at)
            .bind(&recovery_cursor.last_admission_stage)
            .bind(&recovery_cursor.last_admission_cause)
            .bind(&recovery_cursor.last_settled_pass_at)
            .bind(recovery_cursor.last_settled_pass_complete)
            .bind(recovery_cursor.last_settled_pass_inspected_entries)
            .bind(recovery_cursor.last_settled_pass_referenced_skipped)
            .bind(recovery_cursor.last_settled_pass_quarantined)
            .bind(recovery_cursor.last_settled_pass_removed)
            .bind(recovery_cursor.last_settled_pass_removed_bytes)
            .bind(&recovery_cursor.last_nonzero_removal_at)
            .bind(recovery_cursor.last_nonzero_removal)
            .bind(recovery_cursor.last_nonzero_removal_bytes)
            .bind(recovery_cursor.updated_at_epoch_ms)
            .execute(&mut *transaction)
            .await
            .context("upsert retention recovery cursor")?;
        }

        for reconciliation in &operation.retention_raw_reconciliation {
            sqlx::query(
                "INSERT INTO retention_raw_reconciliation
                    (raw_path, file_identity_sha256, byte_size, quarantined_at,
                     release_pending, release_pending_at, removal_evidence_persisted,
                     updated_at_epoch_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (raw_path) DO UPDATE SET
                     file_identity_sha256 = EXCLUDED.file_identity_sha256,
                     byte_size = EXCLUDED.byte_size,
                     quarantined_at = EXCLUDED.quarantined_at,
                     release_pending = EXCLUDED.release_pending,
                     release_pending_at = EXCLUDED.release_pending_at,
                     removal_evidence_persisted = EXCLUDED.removal_evidence_persisted,
                     updated_at_epoch_ms = EXCLUDED.updated_at_epoch_ms
                 WHERE retention_raw_reconciliation.updated_at_epoch_ms
                       <= EXCLUDED.updated_at_epoch_ms
                   AND retention_raw_reconciliation.file_identity_sha256
                       = EXCLUDED.file_identity_sha256",
            )
            .bind(&reconciliation.raw_path)
            .bind(&reconciliation.file_identity_sha256)
            .bind(reconciliation.byte_size)
            .bind(&reconciliation.quarantined_at)
            .bind(reconciliation.release_pending)
            .bind(&reconciliation.release_pending_at)
            .bind(reconciliation.removal_evidence_persisted)
            .bind(reconciliation.updated_at_epoch_ms)
            .execute(&mut *transaction)
            .await
            .context("upsert retention Raw reconciliation")?;
            let stored = sqlx::query(
                "SELECT file_identity_sha256
                   FROM retention_raw_reconciliation WHERE raw_path = $1",
            )
            .bind(&reconciliation.raw_path)
            .fetch_one(&mut *transaction)
            .await
            .context("read retention Raw reconciliation after upsert")?;
            let stored_identity: String = stored.try_get("file_identity_sha256")?;
            if stored_identity != reconciliation.file_identity_sha256 {
                bail!("retention Raw reconciliation identity conflicts with existing metadata");
            }
        }

        if let Some(proof) = operation.proof.as_ref() {
            sqlx::query(
                "INSERT INTO archive_proofs
                    (archive_batch_id, manifest_sha256, page_count, row_count, coverage_start_at,
                     coverage_end_at, semantic_sha256, verified_at_epoch_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (archive_batch_id) DO NOTHING",
            )
            .bind(proof.archive_batch_id)
            .bind(&proof.manifest_sha256)
            .bind(proof.page_count)
            .bind(proof.row_count)
            .bind(&proof.coverage_start_at)
            .bind(&proof.coverage_end_at)
            .bind(&proof.semantic_sha256)
            .bind(proof.verified_at_epoch_ms)
            .execute(&mut *transaction)
            .await
            .context("insert archive proof")?;

            let stored_proof = sqlx::query(
                "SELECT manifest_sha256, page_count, row_count, coverage_start_at,
                        coverage_end_at, semantic_sha256, verified_at_epoch_ms
                   FROM archive_proofs WHERE archive_batch_id = $1",
            )
            .bind(proof.archive_batch_id)
            .fetch_one(&mut *transaction)
            .await
            .context("read archive proof after insert")?;
            let stored_proof_value = ArchiveProof {
                archive_batch_id: proof.archive_batch_id,
                manifest_sha256: stored_proof.try_get("manifest_sha256")?,
                page_count: stored_proof.try_get("page_count")?,
                row_count: stored_proof.try_get("row_count")?,
                coverage_start_at: stored_proof.try_get("coverage_start_at")?,
                coverage_end_at: stored_proof.try_get("coverage_end_at")?,
                semantic_sha256: stored_proof.try_get("semantic_sha256")?,
                verified_at_epoch_ms: stored_proof.try_get("verified_at_epoch_ms")?,
            };
            if &stored_proof_value != proof {
                bail!("archive proof conflicts with existing metadata");
            }
        }

        for marker in &operation.replay_markers {
            sqlx::query(
                "INSERT INTO hourly_rollup_archive_replay
                    (target, dataset, file_path, archive_sha256)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (target, dataset, file_path) DO UPDATE SET
                     replayed_at = CURRENT_TIMESTAMP
                 WHERE hourly_rollup_archive_replay.archive_sha256
                       = EXCLUDED.archive_sha256",
            )
            .bind(&marker.target)
            .bind(&marker.dataset)
            .bind(&marker.file_path)
            .bind(&marker.archive_sha256)
            .execute(&mut *transaction)
            .await
            .context("upsert hourly rollup archive replay marker")?;
            let stored_marker = sqlx::query(
                "SELECT archive_sha256
                   FROM hourly_rollup_archive_replay
                  WHERE target = $1 AND dataset = $2 AND file_path = $3",
            )
            .bind(&marker.target)
            .bind(&marker.dataset)
            .bind(&marker.file_path)
            .fetch_one(&mut *transaction)
            .await
            .context("read hourly rollup archive replay marker after upsert")?;
            let stored_sha: Option<String> = stored_marker.try_get("archive_sha256")?;
            if stored_sha.as_deref() != Some(marker.archive_sha256.as_str()) {
                bail!("archive replay marker conflicts with existing metadata");
            }
        }

        for row in &operation.node_health_rows {
            sqlx::query(
                "INSERT INTO pool_upstream_node_health_archive
                    (archive_file_path, archived_row_id, occurred_at,
                     proxy_binding_key_snapshot, is_success, latency_ms)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (archive_file_path, archived_row_id) DO NOTHING",
            )
            .bind(&operation.manifest.file_path)
            .bind(row.archived_row_id)
            .bind(&row.occurred_at)
            .bind(&row.proxy_binding_key_snapshot)
            .bind(row.is_success)
            .bind(row.latency_ms)
            .execute(&mut *transaction)
            .await
            .context("upsert node-health archive row")?;
            let stored_row = sqlx::query(
                "SELECT occurred_at, proxy_binding_key_snapshot, is_success, latency_ms
                   FROM pool_upstream_node_health_archive
                  WHERE archive_file_path = $1 AND archived_row_id = $2",
            )
            .bind(&operation.manifest.file_path)
            .bind(row.archived_row_id)
            .fetch_one(&mut *transaction)
            .await
            .context("read node-health archive row after upsert")?;
            if stored_row.try_get::<String, _>("occurred_at")? != row.occurred_at
                || stored_row.try_get::<String, _>("proxy_binding_key_snapshot")?
                    != row.proxy_binding_key_snapshot
                || stored_row.try_get::<i64, _>("is_success")? != row.is_success
                || stored_row.try_get::<Option<f64>, _>("latency_ms")? != row.latency_ms
            {
                bail!("node-health archive row conflicts with existing metadata");
            }
        }

        let archive_identity = format!("batch:{}", operation.manifest.archive_batch_id);
        for row in &operation.node_health_hourly_rows {
            sqlx::query(
                "INSERT INTO pool_upstream_node_health_hourly_archive
                    (archive_identity, archive_batch_id, archive_file_path,
                     proxy_binding_key_snapshot, bucket_start_epoch, success_count,
                     failure_count)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (archive_identity, proxy_binding_key_snapshot, bucket_start_epoch)
                 DO NOTHING",
            )
            .bind(&archive_identity)
            .bind(operation.manifest.archive_batch_id)
            .bind(&operation.manifest.file_path)
            .bind(&row.proxy_binding_key_snapshot)
            .bind(row.bucket_start_epoch)
            .bind(row.success_count)
            .bind(row.failure_count)
            .execute(&mut *transaction)
            .await
            .context("upsert node-health hourly archive row")?;
            let stored_row = sqlx::query(
                "SELECT archive_batch_id, archive_file_path, success_count, failure_count
                   FROM pool_upstream_node_health_hourly_archive
                  WHERE archive_identity = $1
                    AND proxy_binding_key_snapshot = $2
                    AND bucket_start_epoch = $3",
            )
            .bind(&archive_identity)
            .bind(&row.proxy_binding_key_snapshot)
            .bind(row.bucket_start_epoch)
            .fetch_one(&mut *transaction)
            .await
            .context("read node-health hourly archive row after upsert")?;
            if stored_row.try_get::<i64, _>("archive_batch_id")?
                != operation.manifest.archive_batch_id
                || stored_row.try_get::<String, _>("archive_file_path")?
                    != operation.manifest.file_path
                || stored_row.try_get::<i64, _>("success_count")? != row.success_count
                || stored_row.try_get::<i64, _>("failure_count")? != row.failure_count
            {
                bail!("node-health hourly archive row conflicts with existing metadata");
            }
        }

        for marker in &operation.rollups {
            sqlx::query(
                "INSERT INTO archive_rollup_markers
                    (archive_batch_id, target, bucket_start_epoch_ms, source_identity_sha256,
                     materialized_rows, complete)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (archive_batch_id, target, bucket_start_epoch_ms) DO UPDATE SET
                     source_identity_sha256 = EXCLUDED.source_identity_sha256,
                     materialized_rows = EXCLUDED.materialized_rows,
                     complete = EXCLUDED.complete
                 WHERE archive_rollup_markers.source_identity_sha256 = EXCLUDED.source_identity_sha256
                   AND NOT archive_rollup_markers.complete
                   AND EXCLUDED.materialized_rows >= archive_rollup_markers.materialized_rows",
            )
            .bind(marker.archive_batch_id)
            .bind(&marker.target)
            .bind(marker.bucket_start_epoch_ms)
            .bind(&marker.source_identity_sha256)
            .bind(marker.materialized_rows)
            .bind(marker.complete)
            .execute(&mut *transaction)
            .await
            .context("upsert archive rollup marker")?;

            let stored_marker = sqlx::query(
                "SELECT source_identity_sha256, materialized_rows, complete
                   FROM archive_rollup_markers
                  WHERE archive_batch_id = $1 AND target = $2
                    AND bucket_start_epoch_ms = $3",
            )
            .bind(marker.archive_batch_id)
            .bind(&marker.target)
            .bind(marker.bucket_start_epoch_ms)
            .fetch_one(&mut *transaction)
            .await
            .context("read archive rollup marker after upsert")?;
            let stored_source_identity: String = stored_marker.try_get("source_identity_sha256")?;
            let stored_materialized_rows: i64 = stored_marker.try_get("materialized_rows")?;
            let stored_complete: bool = stored_marker.try_get("complete")?;
            if stored_source_identity != marker.source_identity_sha256
                || stored_materialized_rows < marker.materialized_rows
                || (marker.complete
                    && (!stored_complete || stored_materialized_rows != marker.materialized_rows))
            {
                bail!("archive rollup marker conflicts with existing metadata");
            }
            if marker.complete {
                sqlx::query(
                    "INSERT INTO hourly_rollup_materialized_buckets
                        (target, bucket_start_epoch_ms, source)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (target, bucket_start_epoch_ms, source) DO NOTHING",
                )
                .bind(&marker.target)
                .bind(marker.bucket_start_epoch_ms)
                .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
                .execute(&mut *transaction)
                .await
                .context("record materialized hourly rollup coverage")?;
            }
        }

        let raw_paths = raw_paths_in_lock_order(
            operation
                .raw_reference_changes
                .iter()
                .map(|change| change.raw_path.as_str()),
        );
        for raw_path in raw_paths {
            // Serialize link/release decisions even when the canonical blob row does not exist.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(raw_path)
                .execute(&mut *transaction)
                .await
                .context("lock canonical Raw ownership path")?;
        }
        for change in &operation.raw_reference_changes {
            let state = match change.action {
                RawReferenceAction::Link => "active",
                RawReferenceAction::Release => "released",
            };
            let raw_identity = change
                .raw_identity_sha256
                .as_deref()
                .context("validated Raw reference is missing its file identity")?;
            let existing_change = sqlx::query(
                "SELECT action, raw_identity_sha256, state
                   FROM archive_raw_reference_changes
                  WHERE archive_batch_id = $1 AND raw_path = $2
                    AND owner_kind = $3 AND owner_id = $4 AND raw_role = $5",
            )
            .bind(change.archive_batch_id)
            .bind(&change.raw_path)
            .bind(&change.owner_kind)
            .bind(change.owner_id)
            .bind(&change.raw_role)
            .fetch_optional(&mut *transaction)
            .await
            .context("read existing archive Raw reference change")?;
            if let Some(existing_change) = existing_change {
                let existing_identity: Option<String> =
                    existing_change.try_get("raw_identity_sha256")?;
                let existing_state: String = existing_change.try_get("state")?;
                if existing_identity.as_deref() != Some(raw_identity) {
                    bail!("archive Raw reference change conflicts with existing metadata");
                }
                if existing_state == "released" {
                    // A retry must not reactivate a terminal release decision. A new owner
                    // event uses a new archive batch identity rather than this retry key.
                    continue;
                }
            }
            if change.action == RawReferenceAction::Link {
                sqlx::query(
                    "INSERT INTO proxy_raw_payload_blobs
                        (raw_path, storage_codec, logical_size_bytes, file_identity_sha256)
                     VALUES ($1, 'identity', 0, $2)
                     ON CONFLICT (raw_path) DO UPDATE SET
                         file_identity_sha256 = COALESCE(
                             proxy_raw_payload_blobs.file_identity_sha256,
                             EXCLUDED.file_identity_sha256
                         ),
                         updated_at = CURRENT_TIMESTAMP
                     WHERE proxy_raw_payload_blobs.file_identity_sha256 IS NULL
                        OR proxy_raw_payload_blobs.file_identity_sha256
                           = EXCLUDED.file_identity_sha256",
                )
                .bind(&change.raw_path)
                .bind(raw_identity)
                .execute(&mut *transaction)
                .await
                .context("upsert canonical Raw blob identity")?;
                let stored_blob = sqlx::query(
                    "SELECT file_identity_sha256
                       FROM proxy_raw_payload_blobs WHERE raw_path = $1",
                )
                .bind(&change.raw_path)
                .fetch_one(&mut *transaction)
                .await
                .context("read canonical Raw blob identity after upsert")?;
                let stored_blob_identity: Option<String> =
                    stored_blob.try_get("file_identity_sha256")?;
                if stored_blob_identity.as_deref() != Some(raw_identity) {
                    bail!("canonical Raw blob identity conflicts with existing metadata");
                }
                // Live links are maintained by the source-table triggers/helpers. Archive
                // ownership stays in archive_raw_reference_changes so an archive release can
                // never remove a still-live owner link for the same path and owner identity.
            }
            if change.action == RawReferenceAction::Release {
                // Lock the parent row before counting. Live-link inserts take a foreign-key
                // key-share lock, so an in-flight source write must finish before the count;
                // otherwise the delete below could cascade away a newly committed live link.
                let parent = sqlx::query(
                    "SELECT raw_path
                       FROM proxy_raw_payload_blobs
                      WHERE raw_path = $1
                      FOR UPDATE",
                )
                .bind(&change.raw_path)
                .fetch_optional(&mut *transaction)
                .await
                .context("lock canonical Raw blob before release")?;
                if parent.is_none() {
                    bail!("cannot release a Raw reference without its blob parent");
                }
                let canonical_blob = sqlx::query(
                    "SELECT b.file_identity_sha256,
                            COUNT(l.id)::BIGINT AS link_count
                       FROM proxy_raw_payload_blobs b
                       LEFT JOIN proxy_raw_payload_blob_links l
                         ON l.raw_path = b.raw_path
                      WHERE b.raw_path = $1
                      GROUP BY b.file_identity_sha256",
                )
                .bind(&change.raw_path)
                .fetch_one(&mut *transaction)
                .await
                .context("check canonical Raw blob before release")?;
                let canonical_identity: Option<String> =
                    canonical_blob.try_get("file_identity_sha256")?;
                let canonical_link_count: i64 = canonical_blob.try_get("link_count")?;
                let identity_conflicts = match canonical_identity.as_deref() {
                    Some(identity) => identity != raw_identity,
                    None => canonical_link_count != 0,
                };
                if identity_conflicts {
                    bail!("canonical Raw release identity conflicts with active metadata");
                }
                let active_identity = sqlx::query(
                    "SELECT COUNT(*)::BIGINT AS active_count,
                            COUNT(raw_identity_sha256)::BIGINT AS known_identity_count,
                            COUNT(DISTINCT raw_identity_sha256)::BIGINT AS identity_count,
                            MIN(raw_identity_sha256) AS raw_identity_sha256
                       FROM archive_raw_reference_changes
                      WHERE raw_path = $1 AND owner_kind = $2 AND owner_id = $3
                        AND raw_role = $4 AND action = 'link' AND state = 'active'",
                )
                .bind(&change.raw_path)
                .bind(&change.owner_kind)
                .bind(change.owner_id)
                .bind(&change.raw_role)
                .fetch_one(&mut *transaction)
                .await
                .context("check active archive Raw reference before release")?;
                let active_count: i64 = active_identity.try_get("active_count")?;
                let known_identity_count: i64 = active_identity.try_get("known_identity_count")?;
                let identity_count: i64 = active_identity.try_get("identity_count")?;
                let stored_identity: Option<String> =
                    active_identity.try_get("raw_identity_sha256")?;
                if active_count != 0
                    && (known_identity_count != active_count
                        || identity_count != 1
                        || stored_identity.as_deref() != Some(raw_identity))
                {
                    bail!("archive Raw release identity conflicts with active metadata");
                }
                if active_count != 0 {
                    sqlx::query(
                        "UPDATE archive_raw_reference_changes
                            SET action = 'release', state = 'released'
                          WHERE raw_path = $1
                            AND owner_kind = $2 AND owner_id = $3
                            AND raw_role = $4 AND action = 'link' AND state = 'active'",
                    )
                    .bind(&change.raw_path)
                    .bind(&change.owner_kind)
                    .bind(change.owner_id)
                    .bind(&change.raw_role)
                    .execute(&mut *transaction)
                    .await
                    .context("release active archive Raw references")?;
                }
                let ownership = sqlx::query(&format!(
                    "SELECT
                                (SELECT COUNT(*)::BIGINT
                                   FROM proxy_raw_payload_blob_links
                                  WHERE raw_path = $1) AS live_links,
                                (SELECT COUNT(*)::BIGINT
                                   FROM archive_raw_reference_changes
                                  WHERE raw_path = $1 AND action = 'link' AND state = 'active')
                                    AS archive_links,
                                (SELECT COUNT(*)::BIGINT
                                   FROM ({ACTIVE_RAW_REFERENCE_UNION}) AS active_raw_references)
                                    AS distinct_links"
                ))
                .bind(&change.raw_path)
                .fetch_one(&mut *transaction)
                .await
                .context("count live and archive Raw references after release")?;
                let ownership = RawOwnershipCounts {
                    live_links: ownership.try_get("live_links")?,
                    archive_links: ownership.try_get("archive_links")?,
                    distinct_links: ownership.try_get("distinct_links")?,
                };
                if ownership.can_remove_blob()? {
                    sqlx::query(
                        "DELETE FROM proxy_raw_payload_blobs
                          WHERE raw_path = $1",
                    )
                    .bind(&change.raw_path)
                    .execute(&mut *transaction)
                    .await
                    .context("release unowned canonical Raw blob metadata")?;
                }
            }
            sqlx::query(
                "INSERT INTO archive_raw_reference_changes
                    (archive_batch_id, raw_path, owner_kind, owner_id, raw_role, action,
                     raw_identity_sha256, state)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (archive_batch_id, raw_path, owner_kind, owner_id, raw_role)
                 DO UPDATE SET
                     action = EXCLUDED.action,
                     raw_identity_sha256 = EXCLUDED.raw_identity_sha256,
                     state = EXCLUDED.state
                 WHERE archive_raw_reference_changes.raw_identity_sha256
                       = EXCLUDED.raw_identity_sha256
                   AND NOT (archive_raw_reference_changes.state = 'released'
                            AND EXCLUDED.action = 'link')",
            )
            .bind(change.archive_batch_id)
            .bind(&change.raw_path)
            .bind(&change.owner_kind)
            .bind(change.owner_id)
            .bind(&change.raw_role)
            .bind(change.action.as_str())
            .bind(raw_identity)
            .bind(state)
            .execute(&mut *transaction)
            .await
            .context("insert archive Raw reference change")?;

            let stored_change = sqlx::query(
                "SELECT action, raw_identity_sha256, state
                   FROM archive_raw_reference_changes
                  WHERE archive_batch_id = $1 AND raw_path = $2
                    AND owner_kind = $3 AND owner_id = $4 AND raw_role = $5",
            )
            .bind(change.archive_batch_id)
            .bind(&change.raw_path)
            .bind(&change.owner_kind)
            .bind(change.owner_id)
            .bind(&change.raw_role)
            .fetch_one(&mut *transaction)
            .await
            .context("read archive Raw reference change after insert")?;
            let stored_action: String = stored_change.try_get("action")?;
            let stored_identity: Option<String> = stored_change.try_get("raw_identity_sha256")?;
            let stored_state: String = stored_change.try_get("state")?;
            let expected_state = match change.action {
                RawReferenceAction::Link => "active",
                RawReferenceAction::Release => "released",
            };
            if stored_action != change.action.as_str()
                || stored_identity != change.raw_identity_sha256
                || stored_state != expected_state
            {
                bail!("archive Raw reference change conflicts with existing metadata");
            }
        }

        for item in &operation.historical_activity {
            let idempotency_key = item.idempotency_key()?;
            sqlx::query(
                "INSERT INTO archive_account_activity_outbox
                    (idempotency_key, archive_batch_id, account_id, last_activity_at,
                     source_revision, control_generation, state)
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending')
                 ON CONFLICT (idempotency_key) DO NOTHING",
            )
            .bind(&idempotency_key)
            .bind(item.archive_batch_id)
            .bind(item.account_id)
            .bind(&item.last_activity_at)
            .bind(item.source_revision)
            .bind(item.control_generation)
            .execute(&mut *transaction)
            .await
            .context("insert historical account activity outbox item")?;

            let stored_item = sqlx::query(
                "SELECT archive_batch_id, account_id, last_activity_at,
                        source_revision, control_generation
                   FROM archive_account_activity_outbox
                  WHERE idempotency_key = $1",
            )
            .bind(&idempotency_key)
            .fetch_one(&mut *transaction)
            .await
            .context("read historical account activity after insert")?;
            if stored_item.try_get::<i64, _>("archive_batch_id")? != item.archive_batch_id
                || stored_item.try_get::<i64, _>("account_id")? != item.account_id
                || stored_item.try_get::<String, _>("last_activity_at")? != item.last_activity_at
                || stored_item.try_get::<i64, _>("source_revision")? != item.source_revision
                || stored_item.try_get::<i64, _>("control_generation")? != item.control_generation
            {
                bail!("historical account activity conflicts with existing metadata");
            }
        }

        transaction
            .commit()
            .await
            .context("commit archive publication")?;
        Ok(ArchivePublishReceipt {
            archive_batch_id: operation.manifest.archive_batch_id,
            activity_items_written: operation.historical_activity.len(),
            raw_reference_changes_written: operation.raw_reference_changes.len(),
        })
    }

    /// Replace the archived account-activity manifest in the same owner as the archive batch.
    /// This records an observation for later control-module delivery; it never calls the control
    /// module and never emits a routing side effect.
    pub(crate) async fn replace_upstream_activity_manifest(
        &self,
        archive_batch_id: i64,
        values: &[HistoricalAccountActivityOutboxItem],
    ) -> Result<usize> {
        let deduplicated = deduplicate_activity_manifest(archive_batch_id, values)?;

        let mut transaction = self.pool.begin().await?;
        let batch_exists: Option<i64> = sqlx::query_scalar(
            "SELECT archive_batch_id FROM archive_batches
              WHERE archive_batch_id = $1 FOR UPDATE",
        )
        .bind(archive_batch_id)
        .fetch_optional(&mut *transaction)
        .await
        .context("lock archive batch before replacing activity manifest")?;
        if batch_exists.is_none() {
            return Ok(0);
        }

        sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = $1")
            .bind(archive_batch_id)
            .execute(&mut *transaction)
            .await
            .context("clear archive account activity manifest")?;
        for (account_id, last_activity_at) in &deduplicated {
            sqlx::query(
                "INSERT INTO archive_batch_upstream_activity
                    (archive_batch_id, account_id, last_activity_at)
                 VALUES ($1, $2, $3)",
            )
            .bind(archive_batch_id)
            .bind(*account_id)
            .bind(&last_activity_at.last_activity_at)
            .execute(&mut *transaction)
            .await
            .context("insert archive account activity manifest")?;

            let idempotency_key = last_activity_at.idempotency_key()?;
            sqlx::query(
                "INSERT INTO archive_account_activity_outbox
                    (idempotency_key, archive_batch_id, account_id, last_activity_at,
                     source_revision, control_generation, state)
                 VALUES ($1, $2, $3, $4, $5, $6, 'pending')
                 ON CONFLICT (idempotency_key) DO NOTHING",
            )
            .bind(idempotency_key)
            .bind(last_activity_at.archive_batch_id)
            .bind(last_activity_at.account_id)
            .bind(&last_activity_at.last_activity_at)
            .bind(last_activity_at.source_revision)
            .bind(last_activity_at.control_generation)
            .execute(&mut *transaction)
            .await
            .context("enqueue refreshed historical account activity")?;
        }
        sqlx::query(
            "UPDATE archive_batches
                SET upstream_activity_manifest_refreshed_at = CURRENT_TIMESTAMP
              WHERE archive_batch_id = $1",
        )
        .bind(archive_batch_id)
        .execute(&mut *transaction)
        .await
        .context("mark archive account activity manifest refreshed")?;
        transaction
            .commit()
            .await
            .context("commit archive account activity manifest")?;
        Ok(deduplicated.len())
    }

    /// Read a bounded batch of observations for the control module to apply. The caller must
    /// acknowledge each item only after the control module has accepted its generation fence.
    pub(crate) async fn read_pending_activity(
        &self,
        limit: u32,
    ) -> Result<Vec<HistoricalAccountActivityOutboxItem>> {
        if limit == 0 {
            bail!("historical account activity read limit must be positive");
        }
        let rows = sqlx::query(
            "SELECT archive_batch_id, account_id, last_activity_at,
                    source_revision, control_generation
               FROM archive_account_activity_outbox
              WHERE state = 'pending'
              ORDER BY archive_batch_id, account_id, idempotency_key
              LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .context("read pending historical account activity")?;
        rows.into_iter()
            .map(|row| {
                let item = HistoricalAccountActivityOutboxItem {
                    archive_batch_id: row.try_get("archive_batch_id")?,
                    account_id: row.try_get("account_id")?,
                    last_activity_at: row.try_get("last_activity_at")?,
                    source_revision: row.try_get("source_revision")?,
                    control_generation: row.try_get("control_generation")?,
                };
                item.validate()?;
                Ok(item)
            })
            .collect()
    }

    pub(crate) async fn upsert_account_activity_v2_bucket_repair_watermarks(
        &self,
        values: &[(i64, i64)],
    ) -> Result<usize> {
        let mut deduplicated = BTreeMap::<i64, i64>::new();
        for (bucket_start_epoch_ms, cursor_id) in values {
            if *bucket_start_epoch_ms < 0 || *cursor_id < 0 {
                bail!("account activity repair watermark cannot be negative");
            }
            deduplicated
                .entry(*bucket_start_epoch_ms)
                .and_modify(|current| *current = (*current).max(*cursor_id))
                .or_insert(*cursor_id);
        }
        let mut transaction = self.pool.begin().await?;
        for (bucket_start_epoch_ms, cursor_id) in &deduplicated {
            sqlx::query(
                "INSERT INTO account_activity_v2_bucket_repair_watermarks
                    (bucket_start_epoch_ms, cursor_id)
                 VALUES ($1, $2)
                 ON CONFLICT (bucket_start_epoch_ms) DO UPDATE SET
                     cursor_id = GREATEST(
                         account_activity_v2_bucket_repair_watermarks.cursor_id,
                         EXCLUDED.cursor_id
                     ),
                     updated_at = CURRENT_TIMESTAMP",
            )
            .bind(*bucket_start_epoch_ms)
            .bind(*cursor_id)
            .execute(&mut *transaction)
            .await
            .context("upsert account activity repair watermark")?;
        }
        transaction
            .commit()
            .await
            .context("commit account activity repair watermarks")?;
        Ok(deduplicated.len())
    }

    pub(crate) async fn read_account_activity_v2_bucket_repair_watermarks(
        &self,
        bucket_start_epoch_ms: &[i64],
    ) -> Result<BTreeMap<i64, i64>> {
        if bucket_start_epoch_ms
            .iter()
            .any(|bucket_start_epoch_ms| *bucket_start_epoch_ms < 0)
        {
            bail!("account activity repair watermark bucket cannot be negative");
        }
        if bucket_start_epoch_ms.is_empty() {
            return Ok(BTreeMap::new());
        }
        let rows = sqlx::query(
            "SELECT bucket_start_epoch_ms, cursor_id
               FROM account_activity_v2_bucket_repair_watermarks
              WHERE bucket_start_epoch_ms = ANY($1)",
        )
        .bind(bucket_start_epoch_ms.to_vec())
        .fetch_all(&self.pool)
        .await
        .context("read account activity repair watermarks")?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("bucket_start_epoch_ms")?,
                    row.try_get("cursor_id")?,
                ))
            })
            .collect()
    }

    /// Acknowledgement is idempotent and is accepted only after the control module reports a
    /// generation at least as new as the observation's generation. It never overwrites control
    /// state; that compare-and-apply belongs to the account-routing module.
    pub(crate) async fn acknowledge_activity(
        &self,
        item: &HistoricalAccountActivityOutboxItem,
        applied_control_generation: i64,
    ) -> Result<bool> {
        item.validate()?;
        if applied_control_generation < item.control_generation {
            bail!("cannot acknowledge activity before its control generation is applied");
        }
        let idempotency_key = item.idempotency_key()?;
        let stored_item = sqlx::query(
            "SELECT archive_batch_id, account_id, last_activity_at,
                    source_revision, control_generation
               FROM archive_account_activity_outbox
              WHERE idempotency_key = $1",
        )
        .bind(&idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .context("read historical account activity before acknowledgement")?;
        let Some(stored_item) = stored_item else {
            return Ok(false);
        };
        if stored_item.try_get::<i64, _>("archive_batch_id")? != item.archive_batch_id
            || stored_item.try_get::<i64, _>("account_id")? != item.account_id
            || stored_item.try_get::<String, _>("last_activity_at")? != item.last_activity_at
            || stored_item.try_get::<i64, _>("source_revision")? != item.source_revision
            || stored_item.try_get::<i64, _>("control_generation")? != item.control_generation
        {
            bail!("historical account activity acknowledgement conflicts with stored metadata");
        }
        let result = sqlx::query(
            "UPDATE archive_account_activity_outbox
                SET state = 'acknowledged', acknowledged_generation = $2
              WHERE idempotency_key = $1
                AND state = 'pending'
                AND control_generation <= $2",
        )
        .bind(idempotency_key)
        .bind(applied_control_generation)
        .execute(&self.pool)
        .await
        .context("acknowledge historical account activity")?;
        Ok(result.rows_affected() == 1)
    }

    /// Prune durable metadata after all proofs and reference checks pass. This method never
    /// deletes an archive or Raw file. Every caller-supplied proof is re-read while the batch is
    /// locked so a stale cleanup decision cannot remove newer migration state.
    pub(crate) async fn prune_metadata(&self, plan: &ArchivePrunePlan) -> Result<bool> {
        let plan = plan.canonicalized(&self.archive_root, &self.raw_root)?;
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(
                        hashtextextended('codex-vibe-monitor:archive-storage-schema', 0)
                    )",
        )
        .execute(&mut *transaction)
        .await
        .context("lock archive storage schema during pruning")?;

        let manifest_row = sqlx::query(
            "SELECT dataset, month_key, day_key, part, file_path, sha256,
                    source_identity_sha256, row_count, layout, codec, cleanup_state,
                    coverage_start_at, coverage_end_at, summary_source_kind,
                    historical_rollups_materialized_at, status
               FROM archive_batches
              WHERE archive_batch_id = $1 FOR UPDATE",
        )
        .bind(plan.manifest.archive_batch_id)
        .fetch_optional(&mut *transaction)
        .await
        .context("lock archive metadata before pruning")?;
        let Some(manifest_row) = manifest_row else {
            return Ok(false);
        };
        let stored_manifest = ArchiveManifest {
            archive_batch_id: plan.manifest.archive_batch_id,
            dataset: manifest_row.try_get("dataset")?,
            month_key: manifest_row.try_get("month_key")?,
            day_key: manifest_row
                .try_get::<Option<String>, _>("day_key")?
                .unwrap_or_default(),
            part: manifest_row.try_get("part")?,
            file_path: manifest_row.try_get("file_path")?,
            sha256: manifest_row.try_get("sha256")?,
            source_identity_sha256: manifest_row.try_get("source_identity_sha256")?,
            row_count: manifest_row.try_get("row_count")?,
            layout: manifest_row.try_get("layout")?,
            codec: manifest_row.try_get("codec")?,
            cleanup_state: manifest_row.try_get("cleanup_state")?,
            summary_source_kind: manifest_row.try_get("summary_source_kind")?,
            coverage_start_at: manifest_row.try_get("coverage_start_at")?,
            coverage_end_at: manifest_row.try_get("coverage_end_at")?,
            historical_rollups_materialized_at: manifest_row
                .try_get("historical_rollups_materialized_at")?,
        };
        let stored_status: String = manifest_row.try_get("status")?;
        if stored_status != ARCHIVE_STATUS_COMPLETED || stored_manifest != plan.manifest {
            return Ok(false);
        }

        let pending_activity_outbox: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM archive_account_activity_outbox
              WHERE archive_batch_id = $1 AND state = 'pending'",
        )
        .bind(plan.manifest.archive_batch_id)
        .fetch_one(&mut *transaction)
        .await
        .context("recheck historical account activity before pruning")?;
        if pending_activity_outbox != 0 {
            return Ok(false);
        }

        if let Some(proof) = plan.proof.as_ref() {
            let proof_row = sqlx::query(
                "SELECT manifest_sha256, page_count, row_count, coverage_start_at,
                        coverage_end_at, semantic_sha256, verified_at_epoch_ms
                   FROM archive_proofs WHERE archive_batch_id = $1",
            )
            .bind(plan.manifest.archive_batch_id)
            .fetch_optional(&mut *transaction)
            .await
            .context("recheck archive proof before pruning")?;
            let Some(proof_row) = proof_row else {
                return Ok(false);
            };
            let stored_proof = ArchiveProof {
                archive_batch_id: proof.archive_batch_id,
                manifest_sha256: proof_row.try_get("manifest_sha256")?,
                page_count: proof_row.try_get("page_count")?,
                row_count: proof_row.try_get("row_count")?,
                coverage_start_at: proof_row.try_get("coverage_start_at")?,
                coverage_end_at: proof_row.try_get("coverage_end_at")?,
                semantic_sha256: proof_row.try_get("semantic_sha256")?,
                verified_at_epoch_ms: proof_row.try_get("verified_at_epoch_ms")?,
            };
            if &stored_proof != proof {
                return Ok(false);
            }
        }

        let raw_paths = raw_paths_in_lock_order(
            plan.raw_reference_checks
                .iter()
                .map(|check| check.raw_path.as_str()),
        );
        for raw_path in &raw_paths {
            // Match publish's lock order so prune cannot race a new Raw ownership link.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(raw_path)
                .execute(&mut *transaction)
                .await
                .context("lock Raw ownership path before pruning")?;
        }
        for raw_path in &raw_paths {
            let parent = sqlx::query_scalar::<_, String>(
                "SELECT raw_path
                   FROM proxy_raw_payload_blobs
                  WHERE raw_path = $1
                  FOR UPDATE",
            )
            .bind(raw_path)
            .fetch_optional(&mut *transaction)
            .await
            .context("lock Raw blob parent before pruning")?;
            if parent.is_none() {
                return Ok(false);
            }
        }

        let stored_rollup_rows = sqlx::query(
            "SELECT target, bucket_start_epoch_ms, source_identity_sha256,
                    materialized_rows, complete
               FROM archive_rollup_markers WHERE archive_batch_id = $1",
        )
        .bind(plan.manifest.archive_batch_id)
        .fetch_all(&mut *transaction)
        .await
        .context("recheck archive rollups before pruning")?;
        let stored_rollups = stored_rollup_rows
            .into_iter()
            .map(|row| {
                Ok::<_, anyhow::Error>((
                    row.try_get::<String, _>("target")?,
                    row.try_get::<i64, _>("bucket_start_epoch_ms")?,
                    row.try_get::<String, _>("source_identity_sha256")?,
                    row.try_get::<i64, _>("materialized_rows")?,
                    row.try_get::<bool, _>("complete")?,
                ))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let expected_rollups = plan
            .rollups
            .iter()
            .map(|marker| {
                (
                    marker.target.clone(),
                    marker.bucket_start_epoch_ms,
                    marker.source_identity_sha256.clone(),
                    marker.materialized_rows,
                    marker.complete,
                )
            })
            .collect::<BTreeSet<_>>();
        if stored_rollups != expected_rollups {
            return Ok(false);
        }

        let stored_raw_path_rows = sqlx::query_scalar::<_, String>(
            "SELECT DISTINCT raw_path FROM archive_raw_reference_changes
              WHERE archive_batch_id = $1 ORDER BY raw_path",
        )
        .bind(plan.manifest.archive_batch_id)
        .fetch_all(&mut *transaction)
        .await
        .context("recheck archive Raw paths before pruning")?;
        let stored_raw_paths = stored_raw_path_rows.into_iter().collect::<BTreeSet<_>>();
        let expected_raw_paths = plan
            .raw_reference_checks
            .iter()
            .map(|check| check.raw_path.clone())
            .collect::<BTreeSet<_>>();
        if stored_raw_paths != expected_raw_paths {
            return Ok(false);
        }

        for check in &plan.raw_reference_checks {
            let active_refs: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*)::BIGINT
                   FROM ({ACTIVE_RAW_REFERENCE_UNION}) AS active_raw_references"
            ))
            .bind(&check.raw_path)
            .fetch_one(&mut *transaction)
            .await
            .context("recheck Raw references before archive pruning")?;
            if active_refs != check.active_reference_count || active_refs != 0 {
                return Ok(false);
            }

            let identity_row = sqlx::query(
                "SELECT COUNT(*)::BIGINT AS identity_rows,
                        COUNT(raw_identity_sha256)::BIGINT AS known_identity_rows,
                        COUNT(DISTINCT raw_identity_sha256)::BIGINT AS identity_count,
                        MIN(raw_identity_sha256) AS raw_identity_sha256
                   FROM (
                        SELECT DISTINCT raw_path, owner_kind, owner_id, raw_role,
                               raw_identity_sha256
                          FROM archive_raw_reference_changes
                         WHERE raw_path = $1
                   ) AS distinct_archive_references",
            )
            .bind(&check.raw_path)
            .fetch_one(&mut *transaction)
            .await
            .context("recheck Raw file identity before archive pruning")?;
            let identity_rows: i64 = identity_row.try_get("identity_rows")?;
            let known_identity_rows: i64 = identity_row.try_get("known_identity_rows")?;
            let identity_count: i64 = identity_row.try_get("identity_count")?;
            let stored_identity: Option<String> = identity_row.try_get("raw_identity_sha256")?;
            if identity_rows != known_identity_rows
                || identity_count != 1
                || stored_identity.as_deref() != Some(check.file_identity_sha256.as_str())
            {
                return Ok(false);
            }
        }

        let prepared_at_epoch_ms = plan
            .raw_reference_checks
            .iter()
            .map(|check| check.checked_at_epoch_ms)
            .max()
            .unwrap_or_default();
        sqlx::query(
            "INSERT INTO archive_prune_ledger
                (archive_batch_id, file_path, manifest_sha256, file_identity_sha256,
                 prepared_at_epoch_ms, metadata_pruned_at_epoch_ms)
             VALUES ($1, $2, $3, $4, $5, NULL)
             ON CONFLICT (archive_batch_id) DO NOTHING",
        )
        .bind(plan.manifest.archive_batch_id)
        .bind(&plan.manifest.file_path)
        .bind(&plan.manifest.sha256)
        .bind(&plan.file_identity_sha256)
        .bind(prepared_at_epoch_ms)
        .execute(&mut *transaction)
        .await
        .context("prepare archive prune ledger")?;
        let ledger_row = sqlx::query(
            "SELECT file_path, manifest_sha256, file_identity_sha256,
                    metadata_pruned_at_epoch_ms
               FROM archive_prune_ledger WHERE archive_batch_id = $1",
        )
        .bind(plan.manifest.archive_batch_id)
        .fetch_one(&mut *transaction)
        .await
        .context("read archive prune ledger")?;
        let ledger_file_path: Option<String> = ledger_row.try_get("file_path")?;
        let ledger_manifest_sha: String = ledger_row.try_get("manifest_sha256")?;
        let ledger_file_identity_sha: String = ledger_row.try_get("file_identity_sha256")?;
        let metadata_pruned_at: Option<i64> = ledger_row.try_get("metadata_pruned_at_epoch_ms")?;
        validate_archive_prune_ledger_identity(
            ledger_file_path.as_deref(),
            &ledger_manifest_sha,
            &ledger_file_identity_sha,
            &plan.manifest.file_path,
            &plan.manifest.sha256,
            &plan.file_identity_sha256,
        )?;
        if metadata_pruned_at.is_some() {
            return Ok(false);
        }

        sqlx::query("DELETE FROM archive_account_activity_outbox WHERE archive_batch_id = $1")
            .bind(plan.manifest.archive_batch_id)
            .execute(&mut *transaction)
            .await
            .context("delete archived account activity outbox metadata")?;
        sqlx::query("DELETE FROM archive_raw_reference_changes WHERE archive_batch_id = $1")
            .bind(plan.manifest.archive_batch_id)
            .execute(&mut *transaction)
            .await
            .context("delete archived Raw reference metadata")?;
        sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = $1")
            .bind(plan.manifest.archive_batch_id)
            .execute(&mut *transaction)
            .await
            .context("delete archived account activity manifest metadata")?;
        sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay
              WHERE dataset = $1 AND file_path = $2",
        )
        .bind(&plan.manifest.dataset)
        .bind(&plan.manifest.file_path)
        .execute(&mut *transaction)
        .await
        .context("delete archive replay metadata")?;
        sqlx::query(
            "DELETE FROM pool_upstream_node_health_hourly_archive
              WHERE archive_batch_id = $1
                 OR (archive_batch_id IS NULL AND archive_file_path = $2)",
        )
        .bind(plan.manifest.archive_batch_id)
        .bind(&plan.manifest.file_path)
        .execute(&mut *transaction)
        .await
        .context("delete node-health hourly archive metadata")?;
        sqlx::query(
            "DELETE FROM pool_upstream_node_health_archive
              WHERE archive_file_path = $1",
        )
        .bind(&plan.manifest.file_path)
        .execute(&mut *transaction)
        .await
        .context("delete node-health archive metadata")?;
        sqlx::query("DELETE FROM archive_rollup_markers WHERE archive_batch_id = $1")
            .bind(plan.manifest.archive_batch_id)
            .execute(&mut *transaction)
            .await
            .context("delete archive rollup metadata")?;
        sqlx::query("DELETE FROM archive_proofs WHERE archive_batch_id = $1")
            .bind(plan.manifest.archive_batch_id)
            .execute(&mut *transaction)
            .await
            .context("delete archive proof metadata")?;
        let deleted_manifest = sqlx::query(
            "DELETE FROM archive_batches
              WHERE archive_batch_id = $1 AND dataset = $2 AND file_path = $3
                AND sha256 = $4 AND cleanup_state = $5 AND status = $6",
        )
        .bind(plan.manifest.archive_batch_id)
        .bind(&plan.manifest.dataset)
        .bind(&plan.manifest.file_path)
        .bind(&plan.manifest.sha256)
        .bind(ARCHIVE_CLEANUP_STATE_DELETE_PENDING)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .execute(&mut *transaction)
        .await
        .context("delete archive manifest metadata")?;
        if deleted_manifest.rows_affected() != 1 {
            return Ok(false);
        }

        let marked = sqlx::query(
            "UPDATE archive_prune_ledger
                SET metadata_pruned_at_epoch_ms = $2
              WHERE archive_batch_id = $1 AND metadata_pruned_at_epoch_ms IS NULL",
        )
        .bind(plan.manifest.archive_batch_id)
        .bind(prepared_at_epoch_ms)
        .execute(&mut *transaction)
        .await
        .context("mark archive metadata as pruned")?;
        transaction
            .commit()
            .await
            .context("commit archive metadata prune")?;
        Ok(marked.rows_affected() == 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyArchiveFormat {
    LegacyMonth,
    SegmentV1,
    SnapshotV2,
}

impl LegacyArchiveFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::LegacyMonth => "legacy_month",
            Self::SegmentV1 => "segment_v1",
            Self::SnapshotV2 => "snapshot_v2",
        }
    }
}

/// Bounded reader description for old SQLite archive files. It emits a fixed projection with
/// NULL fallbacks for optional columns, so historical files are read without rewriting them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LegacyArchiveSchema {
    pub(crate) dataset: String,
    pub(crate) format: LegacyArchiveFormat,
    pub(crate) columns: BTreeSet<String>,
}

impl LegacyArchiveSchema {
    pub(crate) fn new(
        dataset: impl Into<String>,
        format: LegacyArchiveFormat,
        columns: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self> {
        let dataset = dataset.into();
        if dataset.trim().is_empty() {
            bail!("legacy archive dataset cannot be empty");
        }
        let timestamp_column = legacy_timestamp_column(&dataset)?;
        let columns = columns
            .into_iter()
            .map(|column| column.into().trim().to_ascii_lowercase())
            .filter(|column| !column.is_empty())
            .collect::<BTreeSet<_>>();
        for required in ["id", timestamp_column] {
            if !columns.contains(required) {
                bail!("legacy archive is missing required column {required}");
            }
        }
        Ok(Self {
            dataset,
            format,
            columns,
        })
    }

    pub(crate) fn from_manifest(
        manifest: &ArchiveManifest,
        columns: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self> {
        let format = match manifest.layout.as_str() {
            "legacy_month" => LegacyArchiveFormat::LegacyMonth,
            "segment_v1" => LegacyArchiveFormat::SegmentV1,
            "snapshot_v2" => LegacyArchiveFormat::SnapshotV2,
            layout => bail!("unsupported legacy archive layout: {layout}"),
        };
        Self::new(manifest.dataset.clone(), format, columns)
    }

    pub(crate) fn bounded_select_sql(&self, limit: u32) -> Result<String> {
        if limit == 0 {
            bail!("legacy archive read limit must be positive");
        }
        let table = legacy_table_name(&self.dataset)?;
        let timestamp_column = legacy_timestamp_column(&self.dataset)?;
        let optional = |column: &str, fallback: &str| {
            if self.columns.contains(column) {
                format!("\"{column}\"")
            } else {
                fallback.to_owned()
            }
        };
        Ok(format!(
            "SELECT \"id\" AS id, \"{timestamp_column}\" AS occurred_at, {} AS source, {} AS model, {} AS request_raw_path, {} AS response_raw_path FROM \"{table}\" WHERE \"id\" > ?1 ORDER BY \"id\" ASC LIMIT {limit}",
            optional("source", "''"),
            optional("model", "NULL"),
            optional("request_raw_path", "NULL"),
            optional("response_raw_path", "NULL"),
        ))
    }

    pub(crate) fn format_name(&self) -> &'static str {
        self.format.as_str()
    }
}

fn legacy_table_name(dataset: &str) -> Result<&'static str> {
    match dataset {
        "codex_invocations" | "invocations" => Ok(DEFAULT_LEGACY_TABLE),
        "pool_upstream_request_attempts" | "codex_invocation_attempts" | "invocation_attempts" => {
            Ok("pool_upstream_request_attempts")
        }
        "forward_proxy_attempts" => Ok("forward_proxy_attempts"),
        "codex_quota_snapshots" => Ok("codex_quota_snapshots"),
        _ => bail!("unsupported legacy archive dataset: {dataset}"),
    }
}

fn legacy_timestamp_column(dataset: &str) -> Result<&'static str> {
    match dataset {
        "codex_quota_snapshots" => Ok("captured_at"),
        "codex_invocations"
        | "invocations"
        | "pool_upstream_request_attempts"
        | "codex_invocation_attempts"
        | "invocation_attempts"
        | "forward_proxy_attempts" => Ok("occurred_at"),
        _ => bail!("unsupported legacy archive dataset: {dataset}"),
    }
}

fn writer_version_for_layout(layout: &str) -> &'static str {
    match layout {
        "legacy_month" => "legacy_month_v1",
        "segment_v1" => "segment_v1",
        "snapshot_v2" => "snapshot_v2",
        _ => "unknown",
    }
}

fn validate_sha256(label: &str, value: &str) -> Result<()> {
    if value.len() != SHA256_HEX_LENGTH || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} must be a 64-character hexadecimal digest");
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn normalize_archive_path(path: &str, archive_root: &Path) -> Result<String> {
    normalize_bounded_path(path, archive_root, "archive")
}

pub(crate) fn validate_archive_prune_ledger_identity(
    ledger_file_path: Option<&str>,
    ledger_manifest_sha: &str,
    ledger_file_identity_sha: &str,
    expected_file_path: &str,
    expected_manifest_sha: &str,
    expected_file_identity_sha: &str,
) -> Result<()> {
    if ledger_file_path != Some(expected_file_path)
        || ledger_manifest_sha != expected_manifest_sha
        || ledger_file_identity_sha != expected_file_identity_sha
    {
        bail!("archive prune ledger identity or path conflicts with current plan");
    }
    Ok(())
}

fn normalize_raw_path(path: &str, raw_root: &Path) -> Result<String> {
    normalize_bounded_path(path, raw_root, "Raw")
}

fn normalize_bounded_path(path: &str, root: &Path, label: &str) -> Result<String> {
    let path = Path::new(path.trim());
    if path.as_os_str().is_empty() {
        bail!("{label} path cannot be empty");
    }
    let root = lexical_root(root, label)?;
    let relative = if path.is_absolute() {
        path.strip_prefix(&root)
            .with_context(|| format!("{label} path is outside configured root"))?
            .to_owned()
    } else {
        let root_name = root.file_name().and_then(|name| name.to_str());
        if let Some(root_name) = root_name {
            path.strip_prefix(root_name).unwrap_or(path).to_owned()
        } else {
            path.to_owned()
        }
    };
    normalize_relative_path(&relative, label)
}

fn lexical_root(root: &Path, label: &str) -> Result<PathBuf> {
    if root.as_os_str().is_empty() {
        bail!("{label} root cannot be empty");
    }
    let mut normalized = PathBuf::new();
    for component in root.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => bail!("{label} root cannot contain parent traversal"),
            Component::Normal(value) => normalized.push(value),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

fn normalize_relative_path(path: &Path, label: &str) -> Result<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .with_context(|| format!("{label} path is not valid UTF-8"))?;
                if value.is_empty() {
                    bail!("{label} path contains an empty component");
                }
                parts.push(value.to_owned());
            }
            Component::CurDir => {}
            Component::ParentDir => bail!("{label} path contains parent traversal"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("{label} path must resolve below the configured root")
            }
        }
    }
    if parts.is_empty() {
        bail!("{label} path cannot resolve to the root itself");
    }
    Ok(parts.join("/"))
}
