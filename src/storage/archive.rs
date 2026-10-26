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

/// PostgreSQL tables owned by the archive adapter. The list is intentionally narrow so the
/// migration engine and account-routing module cannot accidentally become dependencies of it.
pub(crate) const ARCHIVE_STORAGE_TABLES: &[&str] = &[
    "archive_batches",
    "archive_batch_upstream_activity",
    "archive_rollup_markers",
    "hourly_rollup_archive_replay",
    "hourly_rollup_materialized_buckets",
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
    "CREATE TABLE IF NOT EXISTS proxy_raw_payload_blobs (
        raw_path TEXT PRIMARY KEY,
        storage_codec TEXT NOT NULL DEFAULT 'identity',
        logical_size_bytes BIGINT NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
        updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
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
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
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
        if let (Some(start), Some(end)) = (&self.coverage_start_at, &self.coverage_end_at)
            && start > end
        {
            bail!("archive coverage start is after its end");
        }
        Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawReferenceCheck {
    pub(crate) raw_path: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchivePrunePlan {
    pub(crate) manifest: ArchiveManifest,
    pub(crate) proof: ArchiveProof,
    pub(crate) rollups: Vec<ArchiveRollupMarker>,
    pub(crate) raw_reference_checks: Vec<RawReferenceCheck>,
    pub(crate) pending_activity_outbox: i64,
    pub(crate) file_identity_sha256: String,
}

impl ArchivePrunePlan {
    pub(crate) fn validate(&self, archive_root: &Path, raw_root: &Path) -> Result<()> {
        self.manifest.validate(archive_root)?;
        self.proof.validate_for(&self.manifest)?;
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
        if self.rollups.is_empty() {
            bail!("archive pruning requires at least one rollup marker");
        }
        for marker in &self.rollups {
            marker.validate(&self.manifest)?;
            if !marker.complete {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchivePublishOperation {
    pub(crate) manifest: ArchiveManifest,
    pub(crate) proof: ArchiveProof,
    pub(crate) rollups: Vec<ArchiveRollupMarker>,
    pub(crate) raw_reference_changes: Vec<RawReferenceChange>,
    pub(crate) historical_activity: Vec<HistoricalAccountActivityOutboxItem>,
    pub(crate) retention_prepared_archive: Option<RetentionPreparedArchive>,
    pub(crate) retention_recovery_cursor: Option<RetentionRecoveryCursor>,
    pub(crate) retention_raw_reconciliation: Vec<RetentionRawReconciliation>,
}

impl ArchivePublishOperation {
    pub(crate) fn validate(&self, archive_root: &Path, raw_root: &Path) -> Result<()> {
        self.manifest.validate(archive_root)?;
        self.proof.validate_for(&self.manifest)?;
        if self.rollups.is_empty() {
            bail!("archive publication requires a rollup marker");
        }
        for marker in &self.rollups {
            marker.validate(&self.manifest)?;
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

impl ArchiveStorageAdapter {
    pub(crate) fn new(pool: PgPool, archive_root: PathBuf, raw_root: PathBuf) -> Self {
        Self {
            pool,
            archive_root,
            raw_root,
        }
    }

    pub(crate) async fn ensure_schema(&self) -> Result<()> {
        for statement in ARCHIVE_STORAGE_SCHEMA {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .with_context(|| "create archive storage table")?;
        }
        for statement in ARCHIVE_STORAGE_SCHEMA_UPGRADES {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .with_context(|| "upgrade archive storage table")?;
        }
        Ok(())
    }

    /// Publish all archive metadata in one transaction. The SQL writes metadata and outbox
    /// rows only; no file operation or account-routing notification is performed here.
    pub(crate) async fn publish(
        &self,
        operation: &ArchivePublishOperation,
    ) -> Result<ArchivePublishReceipt> {
        operation.validate(&self.archive_root, &self.raw_root)?;
        let mut transaction = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO archive_batches
                (archive_batch_id, dataset, month_key, day_key, part, file_path, sha256,
                 source_identity_sha256, row_count, layout, codec, cleanup_state,
                 coverage_start_at, coverage_end_at, status, writer_version, summary_source_kind)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                     'completed', $15, 'unknown')
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
        .bind(writer_version_for_layout(&operation.manifest.layout))
        .execute(&mut *transaction)
        .await
        .context("insert archive manifest")?;

        let existing = sqlx::query(
            "SELECT dataset, month_key, day_key, part, file_path, sha256,
                     source_identity_sha256, row_count, status, layout, codec,
                     writer_version, cleanup_state, coverage_start_at, coverage_end_at
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
        {
            bail!("archive batch metadata conflicts with existing metadata");
        }

        if let Some(prepared_archive) = &operation.retention_prepared_archive {
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
                "SELECT file_path, source_identity_sha256
                   FROM retention_prepared_archives WHERE prepared_key = $1",
            )
            .bind(&prepared_archive.prepared_key)
            .fetch_one(&mut *transaction)
            .await
            .context("read retention prepared archive after upsert")?;
            let stored_path: String = stored.try_get("file_path")?;
            let stored_source_identity: String = stored.try_get("source_identity_sha256")?;
            if stored_path != prepared_archive.file_path
                || stored_source_identity != prepared_archive.source_identity_sha256
            {
                bail!("retention prepared archive identity conflicts with existing metadata");
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

        sqlx::query(
            "INSERT INTO archive_proofs
                (archive_batch_id, manifest_sha256, page_count, row_count, coverage_start_at,
                 coverage_end_at, semantic_sha256, verified_at_epoch_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (archive_batch_id) DO NOTHING",
        )
        .bind(operation.proof.archive_batch_id)
        .bind(&operation.proof.manifest_sha256)
        .bind(operation.proof.page_count)
        .bind(operation.proof.row_count)
        .bind(&operation.proof.coverage_start_at)
        .bind(&operation.proof.coverage_end_at)
        .bind(&operation.proof.semantic_sha256)
        .bind(operation.proof.verified_at_epoch_ms)
        .execute(&mut *transaction)
        .await
        .context("insert archive proof")?;

        let stored_proof = sqlx::query(
            "SELECT manifest_sha256, page_count, row_count, coverage_start_at,
                    coverage_end_at, semantic_sha256, verified_at_epoch_ms
               FROM archive_proofs WHERE archive_batch_id = $1",
        )
        .bind(operation.proof.archive_batch_id)
        .fetch_one(&mut *transaction)
        .await
        .context("read archive proof after insert")?;
        let stored_proof_value = ArchiveProof {
            archive_batch_id: operation.proof.archive_batch_id,
            manifest_sha256: stored_proof.try_get("manifest_sha256")?,
            page_count: stored_proof.try_get("page_count")?,
            row_count: stored_proof.try_get("row_count")?,
            coverage_start_at: stored_proof.try_get("coverage_start_at")?,
            coverage_end_at: stored_proof.try_get("coverage_end_at")?,
            semantic_sha256: stored_proof.try_get("semantic_sha256")?,
            verified_at_epoch_ms: stored_proof.try_get("verified_at_epoch_ms")?,
        };
        if stored_proof_value != operation.proof {
            bail!("archive proof conflicts with existing metadata");
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
            if change.action == RawReferenceAction::Release {
                let active_identity = sqlx::query(
                    "SELECT COUNT(*)::BIGINT AS active_count,
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
                let identity_count: i64 = active_identity.try_get("identity_count")?;
                let stored_identity: Option<String> =
                    active_identity.try_get("raw_identity_sha256")?;
                if active_count != 0
                    && (identity_count != 1 || stored_identity.as_deref() != Some(raw_identity))
                {
                    bail!("archive Raw release identity conflicts with active metadata");
                }
                if active_count != 0 {
                    sqlx::query(
                        "UPDATE archive_raw_reference_changes
                            SET action = 'release', state = 'released'
                          WHERE raw_path = $1 AND owner_kind = $2 AND owner_id = $3
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
                       = EXCLUDED.raw_identity_sha256",
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
            .bind(last_activity_at)
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
        plan.validate(&self.archive_root, &self.raw_root)?;
        let mut transaction = self.pool.begin().await?;

        let manifest_row = sqlx::query(
            "SELECT dataset, month_key, day_key, part, file_path, sha256,
                    source_identity_sha256, row_count, layout, codec, cleanup_state,
                    coverage_start_at, coverage_end_at, status
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
            coverage_start_at: manifest_row.try_get("coverage_start_at")?,
            coverage_end_at: manifest_row.try_get("coverage_end_at")?,
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
            archive_batch_id: plan.proof.archive_batch_id,
            manifest_sha256: proof_row.try_get("manifest_sha256")?,
            page_count: proof_row.try_get("page_count")?,
            row_count: proof_row.try_get("row_count")?,
            coverage_start_at: proof_row.try_get("coverage_start_at")?,
            coverage_end_at: proof_row.try_get("coverage_end_at")?,
            semantic_sha256: proof_row.try_get("semantic_sha256")?,
            verified_at_epoch_ms: proof_row.try_get("verified_at_epoch_ms")?,
        };
        if stored_proof != plan.proof {
            return Ok(false);
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
            let active_refs: i64 = sqlx::query_scalar(
                "SELECT COUNT(*)::BIGINT
                   FROM (
                        SELECT raw_path, owner_kind, owner_id, raw_role
                          FROM proxy_raw_payload_blob_links
                         WHERE raw_path = $1
                        UNION
                        SELECT raw_path, owner_kind, owner_id, raw_role
                          FROM archive_raw_reference_changes
                         WHERE raw_path = $1
                           AND action = 'link'
                           AND state = 'active'
                   ) AS active_raw_references",
            )
            .bind(&check.raw_path)
            .fetch_one(&mut *transaction)
            .await
            .context("recheck Raw references before archive pruning")?;
            if active_refs != check.active_reference_count || active_refs != 0 {
                return Ok(false);
            }

            let identity_row = sqlx::query(
                "SELECT COUNT(DISTINCT raw_identity_sha256)::BIGINT AS identity_count,
                        MIN(raw_identity_sha256) AS raw_identity_sha256
                   FROM archive_raw_reference_changes
                  WHERE raw_path = $1 AND raw_identity_sha256 IS NOT NULL",
            )
            .bind(&check.raw_path)
            .fetch_one(&mut *transaction)
            .await
            .context("recheck Raw file identity before archive pruning")?;
            let identity_count: i64 = identity_row.try_get("identity_count")?;
            let stored_identity: Option<String> = identity_row.try_get("raw_identity_sha256")?;
            if identity_count != 1
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
                (archive_batch_id, manifest_sha256, file_identity_sha256,
                 prepared_at_epoch_ms, metadata_pruned_at_epoch_ms)
             VALUES ($1, $2, $3, $4, NULL)
             ON CONFLICT (archive_batch_id) DO NOTHING",
        )
        .bind(plan.manifest.archive_batch_id)
        .bind(&plan.manifest.sha256)
        .bind(&plan.file_identity_sha256)
        .bind(prepared_at_epoch_ms)
        .execute(&mut *transaction)
        .await
        .context("prepare archive prune ledger")?;
        let ledger_row = sqlx::query(
            "SELECT manifest_sha256, file_identity_sha256, metadata_pruned_at_epoch_ms
               FROM archive_prune_ledger WHERE archive_batch_id = $1",
        )
        .bind(plan.manifest.archive_batch_id)
        .fetch_one(&mut *transaction)
        .await
        .context("read archive prune ledger")?;
        let ledger_manifest_sha: String = ledger_row.try_get("manifest_sha256")?;
        let ledger_file_identity_sha: String = ledger_row.try_get("file_identity_sha256")?;
        let metadata_pruned_at: Option<i64> = ledger_row.try_get("metadata_pruned_at_epoch_ms")?;
        if ledger_manifest_sha != plan.manifest.sha256
            || ledger_file_identity_sha != plan.file_identity_sha256
        {
            bail!("archive prune ledger identity conflicts with current plan");
        }
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
