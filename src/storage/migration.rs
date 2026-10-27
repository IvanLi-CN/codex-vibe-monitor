//! Bounded online migration contracts and execution primitives.
//!
//! The engine keeps source writes authoritative until a durable authority manifest is published.
//! SQLx-backed production adapters live in `migration_sqlx`; the in-memory implementations here
//! are deterministic reference implementations for the protocol tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::core::{
    AuthorityManifestStore, AuthorityPublication, AuthorityRecoveryContext,
    AuthorityRecoveryRecord, BackendKind, MigrationCheckpoint, MigrationMetadataStore,
    MigrationOutboxItem, MigrationReservation, MigrationState, MigrationStatus, ModuleAuthority,
    ModuleAuthorityManifest, ModuleGenerationGate, StorageModule,
};

pub(crate) const DEFAULT_COPY_MAX_ROWS: usize = 512;
pub(crate) const DEFAULT_COPY_MAX_BYTES: usize = 2 * 1024 * 1024;
pub(crate) const DEFAULT_SOURCE_READ_BUDGET: Duration = Duration::from_millis(50);
pub(crate) const DEFAULT_BATCH_YIELD: Duration = Duration::from_millis(25);
pub(crate) const DEFAULT_MAX_QUEUED_OPERATIONS: usize = 128;
pub(crate) const DEFAULT_MIN_SOURCE_FREE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_CAPTURE_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const INITIAL_CAPTURE_GENERATION: u64 = 1;
pub(crate) const INITIAL_AUTHORITY_GENERATION: u64 = 1;
pub(crate) const DEFAULT_RETRY_INITIAL: Duration = Duration::from_secs(1);
pub(crate) const DEFAULT_RETRY_MAX: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ChangeKind {
    Insert,
    Update,
    Delete,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChangeInput {
    pub(crate) table_name: String,
    pub(crate) stable_key: String,
    pub(crate) previous_key: Option<String>,
    pub(crate) kind: ChangeKind,
    pub(crate) payload: Option<Vec<u8>>,
}

impl ChangeInput {
    pub(crate) fn insert(
        table_name: impl Into<String>,
        stable_key: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            table_name: table_name.into(),
            stable_key: stable_key.into(),
            previous_key: None,
            kind: ChangeKind::Insert,
            payload: Some(payload.into()),
        }
    }

    pub(crate) fn update(
        table_name: impl Into<String>,
        stable_key: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            table_name: table_name.into(),
            stable_key: stable_key.into(),
            previous_key: None,
            kind: ChangeKind::Update,
            payload: Some(payload.into()),
        }
    }

    pub(crate) fn delete(table_name: impl Into<String>, stable_key: impl Into<String>) -> Self {
        Self {
            table_name: table_name.into(),
            stable_key: stable_key.into(),
            previous_key: None,
            kind: ChangeKind::Delete,
            payload: None,
        }
    }

    pub(crate) fn replace(
        table_name: impl Into<String>,
        previous_key: impl Into<String>,
        stable_key: impl Into<String>,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            table_name: table_name.into(),
            stable_key: stable_key.into(),
            previous_key: Some(previous_key.into()),
            kind: ChangeKind::Replace,
            payload: Some(payload.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CapturedChange {
    pub(crate) migration_id: String,
    pub(crate) source_identity: String,
    pub(crate) table_name: String,
    pub(crate) stable_key: String,
    pub(crate) previous_key: Option<String>,
    pub(crate) kind: ChangeKind,
    pub(crate) source_version: i64,
    pub(crate) payload: Option<Vec<u8>>,
    pub(crate) payload_digest: String,
}

impl CapturedChange {
    pub(crate) fn from_input(
        migration_id: &str,
        source_identity: &str,
        source_version: i64,
        input: ChangeInput,
    ) -> Result<Self> {
        if migration_id.trim().is_empty() || source_identity.trim().is_empty() {
            bail!("captured change requires migration and source identities");
        }
        validate_change_input(&input)?;
        if source_version < 0 {
            bail!("source version must not be negative");
        }
        let payload_digest = payload_digest(input.payload.as_deref());
        Ok(Self {
            migration_id: migration_id.to_owned(),
            source_identity: source_identity.to_owned(),
            table_name: input.table_name,
            stable_key: input.stable_key,
            previous_key: input.previous_key,
            kind: input.kind,
            source_version,
            payload: input.payload,
            payload_digest,
        })
    }

    pub(crate) fn validate_for(&self, migration_id: &str, source_identity: &str) -> Result<()> {
        if self.migration_id != migration_id || self.source_identity != source_identity {
            bail!("captured change belongs to another migration identity");
        }
        if self.source_version < 0 {
            bail!("source version must not be negative");
        }
        validate_change_input(&ChangeInput {
            table_name: self.table_name.clone(),
            stable_key: self.stable_key.clone(),
            previous_key: self.previous_key.clone(),
            kind: self.kind,
            payload: self.payload.clone(),
        })?;
        if self.payload_digest != payload_digest(self.payload.as_deref()) {
            bail!("captured change payload digest does not match its payload");
        }
        Ok(())
    }
}

fn validate_change_input(input: &ChangeInput) -> Result<()> {
    if input.table_name.trim().is_empty() || input.stable_key.trim().is_empty() {
        bail!("a source change requires a table and stable key");
    }
    if matches!(
        input.kind,
        ChangeKind::Insert | ChangeKind::Update | ChangeKind::Replace
    ) && input.payload.is_none()
    {
        bail!("a non-delete source change requires a payload");
    }
    if matches!(input.kind, ChangeKind::Replace)
        && input.previous_key.as_deref().is_none_or(str::is_empty)
    {
        bail!("a replace change requires its previous stable key");
    }
    Ok(())
}

fn payload_digest(payload: Option<&[u8]>) -> String {
    let mut digest = Sha256::new();
    match payload {
        Some(payload) => {
            digest.update([1]);
            digest.update((payload.len() as u64).to_be_bytes());
            digest.update(payload);
        }
        None => digest.update([0]),
    }
    encode_hex(&digest.finalize())
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CaptureCheckpoint {
    pub(crate) migration_id: String,
    pub(crate) source_identity: String,
    pub(crate) capture_generation: u64,
    pub(crate) next_version: i64,
    pub(crate) captured_bytes: u64,
    pub(crate) max_capture_bytes: u64,
    pub(crate) continuity_lost: bool,
    pub(crate) baseline_required: bool,
}

impl CaptureCheckpoint {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.migration_id.trim().is_empty() || self.source_identity.trim().is_empty() {
            bail!("capture checkpoint requires migration and source identities");
        }
        if self.capture_generation == 0 || self.next_version < 0 {
            bail!("capture checkpoint has an invalid generation or version");
        }
        if self.max_capture_bytes == 0 {
            bail!("capture checkpoint has no bounded storage capacity");
        }
        if self.continuity_lost && !self.baseline_required {
            bail!("a continuity-lost capture checkpoint requires a new baseline");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SourceWriteAuthority {
    pub(crate) generation: u64,
    pub(crate) fenced: bool,
}

impl SourceWriteAuthority {
    pub(crate) fn validate(self) -> Result<()> {
        if self.generation == 0 {
            bail!("source write authority requires a non-zero generation");
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct VersionCapture {
    state: Arc<Mutex<VersionCaptureState>>,
    migration_id: String,
    source_identity: String,
    max_capture_bytes: u64,
}

#[derive(Debug, Default)]
struct VersionCaptureState {
    capture_generation: u64,
    next_version: i64,
    captured_bytes: u64,
    changes: Vec<CapturedChange>,
    continuity_lost: bool,
    baseline_required: bool,
}

impl VersionCapture {
    pub(crate) fn new(migration_id: impl Into<String>, source_identity: impl Into<String>) -> Self {
        Self::with_max_capture_bytes(migration_id, source_identity, DEFAULT_MAX_CAPTURE_BYTES)
            .expect("default version capture limit must be valid")
    }

    pub(crate) fn with_max_capture_bytes(
        migration_id: impl Into<String>,
        source_identity: impl Into<String>,
        max_capture_bytes: u64,
    ) -> Result<Self> {
        if max_capture_bytes == 0 {
            bail!("version capture capacity must be positive");
        }
        Ok(Self {
            state: Arc::new(Mutex::new(VersionCaptureState {
                capture_generation: INITIAL_CAPTURE_GENERATION,
                ..VersionCaptureState::default()
            })),
            migration_id: migration_id.into(),
            source_identity: source_identity.into(),
            max_capture_bytes,
        })
    }

    pub(crate) async fn capture_transaction(&self, changes: Vec<ChangeInput>) -> Result<i64> {
        if changes.is_empty() {
            bail!("a source transaction must capture at least one change");
        }
        let mut state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        let version = state
            .next_version
            .checked_add(1)
            .ok_or_else(|| anyhow!("source version exhausted"))?;
        let captured = changes
            .into_iter()
            .map(|change| {
                CapturedChange::from_input(
                    &self.migration_id,
                    &self.source_identity,
                    version,
                    change,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let transaction_bytes = captured.iter().try_fold(0_u64, |total, change| {
            total
                .checked_add(
                    change
                        .payload
                        .as_ref()
                        .map_or(0, |payload| payload.len() as u64),
                )
                .ok_or_else(|| anyhow!("source change capture byte count overflowed"))
        })?;
        let next_bytes = state
            .captured_bytes
            .checked_add(transaction_bytes)
            .ok_or_else(|| anyhow!("source change capture byte count overflowed"))?;
        if next_bytes > self.max_capture_bytes {
            bail!("source change capture exceeds its bounded storage capacity");
        }
        state.next_version = version;
        state.captured_bytes = next_bytes;
        state.changes.extend(captured);
        Ok(version)
    }

    pub(crate) async fn checkpoint(&self) -> Result<CaptureCheckpoint> {
        let state = self.state.lock().await;
        let checkpoint = CaptureCheckpoint {
            migration_id: self.migration_id.clone(),
            source_identity: self.source_identity.clone(),
            capture_generation: state.capture_generation,
            next_version: state.next_version,
            captured_bytes: state.captured_bytes,
            max_capture_bytes: self.max_capture_bytes,
            continuity_lost: state.continuity_lost,
            baseline_required: state.baseline_required,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    pub(crate) async fn start_new_capture_generation(&self) -> Result<CaptureCheckpoint> {
        let mut state = self.state.lock().await;
        state.capture_generation = state
            .capture_generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("source capture generation exhausted"))?;
        state.changes.clear();
        state.captured_bytes = 0;
        state.continuity_lost = false;
        state.baseline_required = true;
        let checkpoint = CaptureCheckpoint {
            migration_id: self.migration_id.clone(),
            source_identity: self.source_identity.clone(),
            capture_generation: state.capture_generation,
            next_version: state.next_version,
            captured_bytes: state.captured_bytes,
            max_capture_bytes: self.max_capture_bytes,
            continuity_lost: state.continuity_lost,
            baseline_required: state.baseline_required,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    pub(crate) async fn mark_baseline_complete(
        &self,
        capture_generation: u64,
        through: i64,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        if capture_generation != state.capture_generation {
            bail!("capture baseline belongs to another generation");
        }
        if state.continuity_lost {
            bail!("capture continuity has been lost; a new generation is required");
        }
        if through < 0 || through > state.next_version {
            bail!("capture baseline watermark is outside the current capture");
        }
        state.baseline_required = false;
        Ok(())
    }

    pub(crate) async fn watermark(&self) -> Result<i64> {
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        Ok(state.next_version)
    }

    pub(crate) async fn changes_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        self.changes_between(after, None, limit).await
    }

    pub(crate) async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        self.changes_between(after, Some(through), limit).await
    }

    async fn changes_between(
        &self,
        after: i64,
        through: Option<i64>,
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        if after < 0 || limit == 0 || through.is_some_and(|value| value < after) {
            bail!("source change cursor and limit are invalid");
        }
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        let mut selected = Vec::new();
        let mut versions = BTreeSet::new();
        for change in state.changes.iter().filter(|change| {
            change.source_version > after
                && through.is_none_or(|watermark| change.source_version <= watermark)
        }) {
            if versions.insert(change.source_version) && versions.len() > limit {
                break;
            }
            if versions.len() <= limit {
                selected.push(change.clone());
            }
        }
        Ok(selected)
    }

    pub(crate) async fn mark_continuity_lost(&self) {
        let mut state = self.state.lock().await;
        state.continuity_lost = true;
        state.baseline_required = true;
    }

    pub(crate) fn migration_id(&self) -> &str {
        &self.migration_id
    }

    pub(crate) fn source_identity(&self) -> &str {
        &self.source_identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VersionedRow {
    pub(crate) stable_key: String,
    pub(crate) source_version: i64,
    pub(crate) payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourcePage {
    pub(crate) rows: Vec<VersionedRow>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceRangeSnapshot {
    pub(crate) source_watermark: i64,
    pub(crate) rows: Vec<VersionedRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceRangePage {
    pub(crate) source_watermark: i64,
    pub(crate) page: SourcePage,
}

impl SourcePage {
    fn validate(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<()> {
        self.validate_range(table_name, cursor, None, upper_bound, limits)
    }

    fn validate_range(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<()> {
        if self.rows.len() > limits.max_rows {
            bail!("source page exceeds its row budget");
        }
        let bytes = self
            .rows
            .iter()
            .map(|row| row.payload.len())
            .try_fold(0_usize, usize::checked_add)
            .ok_or_else(|| anyhow!("source page byte count overflowed"))?;
        if bytes > limits.max_bytes {
            bail!("source page exceeds its byte budget");
        }
        if self.rows.is_empty() {
            if !self.complete || self.next_cursor.is_some() {
                bail!("an empty source page must be complete without a cursor");
            }
            return Ok(());
        }
        let mut previous = cursor;
        for row in &self.rows {
            if row.stable_key.trim().is_empty() || row.source_version < 0 {
                bail!("source page contains an invalid row");
            }
            if previous.is_some_and(|key| row.stable_key.as_str() <= key) {
                bail!("source page rows are not strictly key ordered");
            }
            if cursor.is_none() && lower_bound.is_some_and(|key| row.stable_key.as_str() < key) {
                bail!("source page row precedes its range lower bound");
            }
            if upper_bound.is_some_and(|key| row.stable_key.as_str() > key) {
                bail!("source page row exceeds its scan upper bound");
            }
            previous = Some(row.stable_key.as_str());
        }
        if self.next_cursor.as_deref() != self.rows.last().map(|row| row.stable_key.as_str()) {
            bail!("source page cursor does not match its final row");
        }
        // The fixed upper-bound row may be deleted while copying. A complete page means the
        // source observed no more rows in the planned range; it need not end on that key.
        if !self.complete && self.next_cursor.as_deref() == cursor {
            bail!("an incomplete source page must advance its cursor");
        }
        if table_name.trim().is_empty() {
            bail!("source page requires a table name");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TableCopyPlan {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) scan_lower_bound: Option<String>,
    pub(crate) scan_upper_bound: Option<String>,
    pub(crate) capture_start: i64,
}

impl TableCopyPlan {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_persisted_identity(
            "migration plan",
            &self.migration_id,
            &self.installation_id,
            &self.source_identity,
            &self.target_identity,
        )?;
        if self.table_name.trim().is_empty() || self.capture_start < 0 {
            bail!("migration plan has an invalid table or capture boundary");
        }
        validate_table_bounds(
            "migration plan",
            &self.scan_lower_bound,
            &self.scan_upper_bound,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CopyLimits {
    pub(crate) max_rows: usize,
    pub(crate) max_bytes: usize,
    pub(crate) source_read_budget: Duration,
    pub(crate) batch_yield: Duration,
}

impl Default for CopyLimits {
    fn default() -> Self {
        Self {
            max_rows: DEFAULT_COPY_MAX_ROWS,
            max_bytes: DEFAULT_COPY_MAX_BYTES,
            source_read_budget: DEFAULT_SOURCE_READ_BUDGET,
            batch_yield: DEFAULT_BATCH_YIELD,
        }
    }
}

impl CopyLimits {
    pub(crate) fn validate(self) -> Result<()> {
        if self.max_rows == 0 || self.max_bytes == 0 || self.source_read_budget.is_zero() {
            bail!("migration copy limits must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChangeBatchLimits {
    pub(crate) max_rows: usize,
    pub(crate) max_bytes: usize,
}

impl Default for ChangeBatchLimits {
    fn default() -> Self {
        Self {
            max_rows: DEFAULT_COPY_MAX_ROWS,
            max_bytes: DEFAULT_COPY_MAX_BYTES,
        }
    }
}

impl ChangeBatchLimits {
    fn validate(self) -> Result<()> {
        if self.max_rows == 0 || self.max_bytes == 0 {
            bail!("migration change batch limits must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EngineCheckpoint {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) cursor: Option<String>,
    pub(crate) scan_upper_bound: Option<String>,
    pub(crate) capture_start: i64,
    pub(crate) captured_through: i64,
    pub(crate) applied_through: i64,
    pub(crate) copied_rows: u64,
    pub(crate) copied_bytes: u64,
    pub(crate) complete: bool,
}

impl EngineCheckpoint {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_persisted_identity(
            "migration checkpoint",
            &self.migration_id,
            &self.installation_id,
            &self.source_identity,
            &self.target_identity,
        )?;
        if self.table_name.trim().is_empty()
            || self.capture_start < 0
            || self.captured_through < 0
            || self.applied_through < 0
            || self.captured_through < self.applied_through
        {
            bail!("migration checkpoint has an invalid identity or watermark");
        }
        Ok(())
    }

    fn as_contract(&self, authority_generation: u64) -> MigrationCheckpoint {
        MigrationCheckpoint {
            migration_id: self.migration_id.clone(),
            table_name: self.table_name.clone(),
            stable_key: self.cursor.clone().unwrap_or_else(|| {
                format!(
                    "__migration_checkpoint__:{}:{}",
                    self.migration_id, self.table_name
                )
            }),
            cursor: self.cursor.clone(),
            captured_through: Some(self.captured_through),
            applied_through: Some(self.applied_through),
            installation_id: self.installation_id.clone(),
            source_identity: self.source_identity.clone(),
            target_identity: self.target_identity.clone(),
            authority_generation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChunkRange {
    pub(crate) first_key: String,
    pub(crate) last_key: String,
}

impl ChunkRange {
    pub(crate) fn new(first_key: impl Into<String>, last_key: impl Into<String>) -> Result<Self> {
        let range = Self {
            first_key: first_key.into(),
            last_key: last_key.into(),
        };
        if range.first_key.is_empty() || range.last_key.is_empty() {
            bail!("migration chunk range requires both stable keys");
        }
        if range.first_key > range.last_key {
            bail!("migration chunk range is reversed");
        }
        Ok(range)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChunkProof {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) range: ChunkRange,
    pub(crate) source_watermark: i64,
    pub(crate) row_count: u64,
    pub(crate) byte_count: u64,
    pub(crate) digest: String,
}

impl ChunkProof {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_persisted_identity(
            "migration chunk proof",
            &self.migration_id,
            &self.installation_id,
            &self.source_identity,
            &self.target_identity,
        )?;
        if self.source_watermark < 0 || self.range.first_key > self.range.last_key {
            bail!("migration chunk proof has an invalid range or watermark");
        }
        if self.digest.trim().is_empty() {
            bail!("migration chunk proof requires a digest");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TableCoverageProof {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) source_watermark: i64,
    pub(crate) range: Option<ChunkRange>,
    pub(crate) source_lower_bound: Option<String>,
    pub(crate) source_upper_bound: Option<String>,
    pub(crate) target_lower_bound: Option<String>,
    pub(crate) target_upper_bound: Option<String>,
    pub(crate) source_row_count: u64,
    pub(crate) target_row_count: u64,
    pub(crate) source_byte_count: u64,
    pub(crate) target_byte_count: u64,
    pub(crate) source_digest: String,
    pub(crate) target_digest: String,
}

impl TableCoverageProof {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_persisted_identity(
            "table coverage proof",
            &self.migration_id,
            &self.installation_id,
            &self.source_identity,
            &self.target_identity,
        )?;
        if self.table_name.trim().is_empty() {
            bail!("table coverage proof requires a table identity");
        }
        if self.source_watermark < 0 {
            bail!("table coverage proof watermark must not be negative");
        }
        let (first, last) = union_table_bounds(
            &self.source_lower_bound,
            &self.source_upper_bound,
            &self.target_lower_bound,
            &self.target_upper_bound,
        )?;
        match (first, last, &self.range) {
            (None, None, None) => {}
            (Some(first), Some(last), Some(range))
                if first == range.first_key && last == range.last_key => {}
            _ => bail!("table coverage proof range does not match its table bounds"),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CoverageCheckpoint {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) lower_bound: Option<String>,
    pub(crate) upper_bound: Option<String>,
    pub(crate) cursor: Option<String>,
    pub(crate) source_watermark: i64,
    pub(crate) source_row_count: u64,
    pub(crate) target_row_count: u64,
    pub(crate) source_byte_count: u64,
    pub(crate) target_byte_count: u64,
    pub(crate) source_digest: String,
    pub(crate) target_digest: String,
    pub(crate) complete: bool,
}

impl CoverageCheckpoint {
    pub(crate) fn validate(&self) -> Result<()> {
        validate_persisted_identity(
            "coverage checkpoint",
            &self.migration_id,
            &self.installation_id,
            &self.source_identity,
            &self.target_identity,
        )?;
        if self.table_name.trim().is_empty() {
            bail!("coverage checkpoint requires a table identity");
        }
        if self.source_watermark < 0 {
            bail!("coverage checkpoint watermark must not be negative");
        }
        validate_table_bounds("coverage checkpoint", &self.lower_bound, &self.upper_bound)?;
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.trim().is_empty())
        {
            bail!("coverage checkpoint cursor must not be empty");
        }
        if !self.complete && self.cursor.is_none() {
            bail!("an incomplete coverage checkpoint requires a cursor");
        }
        if let Some(cursor) = &self.cursor
            && (self
                .lower_bound
                .as_ref()
                .is_some_and(|lower| cursor < lower)
                || self
                    .upper_bound
                    .as_ref()
                    .is_some_and(|upper| cursor > upper))
        {
            bail!("coverage checkpoint cursor is outside its table bounds");
        }
        Ok(())
    }
}

fn validate_persisted_identity(
    label: &str,
    migration_id: &str,
    installation_id: &str,
    source_identity: &str,
    target_identity: &str,
) -> Result<()> {
    if migration_id.trim().is_empty()
        || installation_id.trim().is_empty()
        || source_identity.trim().is_empty()
        || target_identity.trim().is_empty()
    {
        bail!("{label} requires migration, installation, source and target identities");
    }
    Ok(())
}

fn validate_table_bounds(
    label: &str,
    lower: &Option<String>,
    upper: &Option<String>,
) -> Result<()> {
    if lower.is_some() != upper.is_some() {
        bail!("{label} table bounds must include both lower and upper keys");
    }
    if lower.as_ref().is_some_and(|value| value.trim().is_empty())
        || upper.as_ref().is_some_and(|value| value.trim().is_empty())
    {
        bail!("{label} table bounds must not contain empty keys");
    }
    if let (Some(lower), Some(upper)) = (lower, upper)
        && lower > upper
    {
        bail!("{label} table bounds are reversed");
    }
    Ok(())
}

fn union_table_bounds(
    source_lower: &Option<String>,
    source_upper: &Option<String>,
    target_lower: &Option<String>,
    target_upper: &Option<String>,
) -> Result<(Option<String>, Option<String>)> {
    validate_table_bounds("source", source_lower, source_upper)?;
    validate_table_bounds("target", target_lower, target_upper)?;
    let first = source_lower
        .iter()
        .chain(target_lower.iter())
        .min()
        .cloned();
    let last = source_upper
        .iter()
        .chain(target_upper.iter())
        .max()
        .cloned();
    Ok((first, last))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChunkVerification {
    Verified,
    Dirty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationProgress {
    pub(crate) table_name: String,
    pub(crate) copied_rows: u64,
    pub(crate) copied_bytes: u64,
    pub(crate) cursor: Option<String>,
    pub(crate) applied_through: i64,
    pub(crate) pending_changes: usize,
    pub(crate) complete: bool,
}

#[async_trait]
pub(crate) trait MigrationSource: Send + Sync {
    async fn capture_watermark(&self) -> Result<i64>;
    async fn capture_checkpoint(&self) -> Result<CaptureCheckpoint>;
    async fn mark_baseline_complete(&self, capture_generation: u64, through: i64) -> Result<()>;
    async fn fence_source_writes(
        &self,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<()>;
    async fn reopen_source_writes(&self, expected_generation: u64) -> Result<()>;
    async fn module_tables(&self, module: StorageModule) -> Result<Vec<String>>;
    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)>;
    async fn scan_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage>;
    async fn scan_range_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourceRangePage>;
    async fn rows_in_range_snapshot(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<SourceRangeSnapshot>;
    async fn changes_after(
        &self,
        after: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>>;
    async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>>;
    async fn release_capture_through(&self, through: i64) -> Result<()>;
    fn migration_id(&self) -> &str;
    fn source_identity(&self) -> &str;
}

#[async_trait]
pub(crate) trait MigrationTarget: Send + Sync {
    async fn known_tables(&self, migration_id: &str) -> Result<Vec<String>>;
    async fn load_plan(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCopyPlan>>;
    async fn save_plan(&self, plan: &TableCopyPlan) -> Result<()>;
    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<EngineCheckpoint>>;
    async fn apply_baseline_page(
        &self,
        plan: &TableCopyPlan,
        page: &SourcePage,
        checkpoint: &EngineCheckpoint,
    ) -> Result<()>;
    async fn apply_change_batch(
        &self,
        migration_id: &str,
        source_identity: &str,
        changes: &[CapturedChange],
        checkpoint: &EngineCheckpoint,
    ) -> Result<()>;
    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>>;
    async fn scan_range_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage>;
    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)>;
    async fn save_chunk_proof(&self, proof: &ChunkProof) -> Result<()>;
    async fn verified_chunks(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Vec<ChunkProof>>;
    async fn save_coverage_checkpoint(&self, checkpoint: &CoverageCheckpoint) -> Result<()>;
    async fn load_coverage_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<CoverageCheckpoint>>;
    async fn save_table_coverage_proof(&self, proof: &TableCoverageProof) -> Result<()>;
    async fn load_table_coverage_proof(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCoverageProof>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MigrationPressure {
    pub(crate) source_free_bytes: u64,
    pub(crate) capture_bytes: u64,
    pub(crate) p1_wait_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MigrationLimits {
    pub(crate) copy: CopyLimits,
    pub(crate) change_batch: ChangeBatchLimits,
    pub(crate) max_queued_operations: usize,
    pub(crate) min_source_free_bytes: u64,
    pub(crate) max_capture_bytes: u64,
}

impl Default for MigrationLimits {
    fn default() -> Self {
        Self {
            copy: CopyLimits::default(),
            change_batch: ChangeBatchLimits::default(),
            max_queued_operations: DEFAULT_MAX_QUEUED_OPERATIONS,
            min_source_free_bytes: DEFAULT_MIN_SOURCE_FREE_BYTES,
            max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
        }
    }
}

impl MigrationLimits {
    fn validate(self) -> Result<()> {
        self.copy.validate()?;
        self.change_batch.validate()?;
        if self.max_queued_operations == 0
            || self.min_source_free_bytes == 0
            || self.max_capture_bytes == 0
        {
            bail!("migration pressure limits must be positive");
        }
        Ok(())
    }

    pub(crate) fn should_pause(self, pressure: MigrationPressure) -> bool {
        pressure.source_free_bytes < self.min_source_free_bytes
            || pressure.capture_bytes >= self.max_capture_bytes
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MigrationRunControl {
    paused: Arc<AtomicBool>,
    changed: Arc<Notify>,
    attempts: Arc<AtomicUsize>,
}

impl Default for MigrationRunControl {
    fn default() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
            changed: Arc::new(Notify::new()),
            attempts: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl MigrationRunControl {
    pub(crate) fn pause(&self) {
        self.paused.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }

    pub(crate) fn resume(&self) {
        self.paused.store(false, Ordering::Release);
        self.changed.notify_waiters();
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub(crate) fn record_failure(&self) -> Duration {
        let attempt = self
            .attempts
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        retry_delay(attempt)
    }

    pub(crate) fn record_success(&self) {
        self.attempts.store(0, Ordering::Release);
    }

    pub(crate) fn retry_attempts(&self) -> usize {
        self.attempts.load(Ordering::Acquire)
    }

    pub(crate) async fn wait_until_resumed(&self) {
        loop {
            if !self.is_paused() {
                return;
            }
            self.changed.notified().await;
        }
    }
}

pub(crate) fn retry_delay(attempt: usize) -> Duration {
    let shift = attempt.saturating_sub(1).min(5) as u32;
    let multiplier = 1_u32 << shift;
    DEFAULT_RETRY_INITIAL
        .checked_mul(multiplier)
        .unwrap_or(DEFAULT_RETRY_MAX)
        .min(DEFAULT_RETRY_MAX)
}

#[derive(Debug, Clone)]
pub(crate) struct SourceWriteFence {
    inner: Arc<SourceWriteFenceState>,
}

#[derive(Debug)]
struct SourceWriteFenceState {
    fenced: AtomicBool,
    active_writes: AtomicUsize,
    changed: Notify,
}

pub(crate) struct SourceWriteLease {
    inner: Arc<SourceWriteFenceState>,
}

impl SourceWriteFence {
    pub(crate) fn new(_generation: u64) -> Self {
        Self {
            inner: Arc::new(SourceWriteFenceState {
                fenced: AtomicBool::new(false),
                active_writes: AtomicUsize::new(0),
                changed: Notify::new(),
            }),
        }
    }

    pub(crate) async fn begin_write(&self) -> Result<SourceWriteLease> {
        if self.inner.fenced.load(Ordering::Acquire) {
            bail!("source writes are fenced for authority switching");
        }
        self.inner.active_writes.fetch_add(1, Ordering::AcqRel);
        if self.inner.fenced.load(Ordering::Acquire) {
            self.inner.active_writes.fetch_sub(1, Ordering::AcqRel);
            self.inner.changed.notify_waiters();
            bail!("source writes are fenced for authority switching");
        }
        Ok(SourceWriteLease {
            inner: Arc::clone(&self.inner),
        })
    }

    pub(crate) async fn close_and_drain(&self, budget: Duration) -> Result<()> {
        self.inner.fenced.store(true, Ordering::Release);
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if self.inner.active_writes.load(Ordering::Acquire) == 0 {
                return Ok(());
            }
            let notified = self.inner.changed.notified();
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                self.reopen().await;
                bail!("timed out draining source writes before authority switch");
            }
        }
    }

    pub(crate) async fn reopen(&self) {
        self.inner.fenced.store(false, Ordering::Release);
        self.inner.changed.notify_waiters();
    }

    pub(crate) async fn is_fenced(&self) -> bool {
        self.inner.fenced.load(Ordering::Acquire)
    }
}

impl SourceWriteLease {
    pub(crate) fn complete(self) {}
}

impl Drop for SourceWriteLease {
    fn drop(&mut self) {
        self.inner.active_writes.fetch_sub(1, Ordering::AcqRel);
        self.inner.changed.notify_waiters();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InMemoryMigrationSource {
    migration_id: String,
    source_identity: String,
    state: Arc<Mutex<InMemorySourceState>>,
    verification_delay: Arc<Mutex<Duration>>,
    verification_started: Arc<Notify>,
}

#[derive(Debug)]
struct InMemorySourceState {
    rows: BTreeMap<String, BTreeMap<String, VersionedRow>>,
    module_tables: BTreeMap<StorageModule, BTreeSet<String>>,
    changes: Vec<CapturedChange>,
    next_version: i64,
    captured_bytes: u64,
    capture_generation: u64,
    continuity_lost: bool,
    baseline_required: bool,
    authority_generation: u64,
    source_writes_fenced: bool,
}

impl Default for InMemorySourceState {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
            module_tables: BTreeMap::new(),
            changes: Vec::new(),
            next_version: 0,
            captured_bytes: 0,
            capture_generation: INITIAL_CAPTURE_GENERATION,
            continuity_lost: false,
            baseline_required: false,
            authority_generation: INITIAL_AUTHORITY_GENERATION,
            source_writes_fenced: false,
        }
    }
}

impl InMemoryMigrationSource {
    pub(crate) fn new(migration_id: impl Into<String>, source_identity: impl Into<String>) -> Self {
        Self {
            migration_id: migration_id.into(),
            source_identity: source_identity.into(),
            state: Arc::new(Mutex::new(InMemorySourceState::default())),
            verification_delay: Arc::new(Mutex::new(Duration::ZERO)),
            verification_started: Arc::new(Notify::new()),
        }
    }

    pub(crate) async fn set_verification_delay_for_test(&self, delay: Duration) {
        *self.verification_delay.lock().await = delay;
    }

    pub(crate) async fn wait_for_verification_for_test(&self) {
        self.verification_started.notified().await;
    }

    pub(crate) async fn start_new_capture_generation(&self) -> Result<CaptureCheckpoint> {
        let mut state = self.state.lock().await;
        state.capture_generation = state
            .capture_generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("source capture generation exhausted"))?;
        state.changes.clear();
        state.captured_bytes = 0;
        state.continuity_lost = false;
        state.baseline_required = true;
        self.capture_checkpoint_from_state(&state)
    }

    pub(crate) async fn mark_baseline_complete(
        &self,
        capture_generation: u64,
        through: i64,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        if capture_generation != state.capture_generation {
            bail!("capture baseline belongs to another generation");
        }
        if state.continuity_lost {
            bail!("capture continuity has been lost; a new generation is required");
        }
        if through < 0 || through > state.next_version {
            bail!("capture baseline watermark is outside the current capture");
        }
        state.baseline_required = false;
        Ok(())
    }

    async fn before_verification_read(&self) {
        self.verification_started.notify_waiters();
        let delay = *self.verification_delay.lock().await;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    pub(crate) async fn register_module_table(&self, table_name: impl Into<String>) -> Result<()> {
        let table_name = table_name.into();
        if table_name.trim().is_empty() {
            bail!("in-memory migration table name must not be empty");
        }
        let mut state = self.state.lock().await;
        state
            .module_tables
            .entry(StorageModule::TrafficArchive)
            .or_default()
            .insert(table_name.clone());
        state.rows.entry(table_name).or_default();
        Ok(())
    }

    pub(crate) async fn write(
        &self,
        fence: &SourceWriteFence,
        changes: Vec<ChangeInput>,
    ) -> Result<i64> {
        self.write_for_generation(fence, INITIAL_AUTHORITY_GENERATION, changes)
            .await
    }

    pub(crate) async fn write_for_generation(
        &self,
        fence: &SourceWriteFence,
        expected_generation: u64,
        changes: Vec<ChangeInput>,
    ) -> Result<i64> {
        let lease = fence.begin_write().await?;
        if changes.is_empty() {
            bail!("a source transaction must capture at least one change");
        }
        let mut state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        if state.source_writes_fenced || state.authority_generation != expected_generation {
            bail!("source writer generation is stale or source writes are fenced");
        }
        let version = state
            .next_version
            .checked_add(1)
            .ok_or_else(|| anyhow!("source version exhausted"))?;
        let captured = changes
            .iter()
            .cloned()
            .map(|change| {
                CapturedChange::from_input(
                    &self.migration_id,
                    &self.source_identity,
                    version,
                    change,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let transaction_bytes = captured.iter().try_fold(0_u64, |total, change| {
            total
                .checked_add(
                    change
                        .payload
                        .as_ref()
                        .map_or(0, |payload| payload.len() as u64),
                )
                .ok_or_else(|| anyhow!("source change capture byte count overflowed"))
        })?;
        let next_bytes = state
            .captured_bytes
            .checked_add(transaction_bytes)
            .ok_or_else(|| anyhow!("source change capture byte count overflowed"))?;
        for change in &captured {
            state
                .module_tables
                .entry(StorageModule::TrafficArchive)
                .or_default()
                .insert(change.table_name.clone());
            let table = state.rows.entry(change.table_name.clone()).or_default();
            match change.kind {
                ChangeKind::Insert | ChangeKind::Update => {
                    table.insert(
                        change.stable_key.clone(),
                        VersionedRow {
                            stable_key: change.stable_key.clone(),
                            source_version: version,
                            payload: change.payload.clone().unwrap_or_default(),
                        },
                    );
                }
                ChangeKind::Delete => {
                    table.remove(&change.stable_key);
                }
                ChangeKind::Replace => {
                    if let Some(previous_key) = &change.previous_key {
                        table.remove(previous_key);
                    }
                    table.insert(
                        change.stable_key.clone(),
                        VersionedRow {
                            stable_key: change.stable_key.clone(),
                            source_version: version,
                            payload: change.payload.clone().unwrap_or_default(),
                        },
                    );
                }
            }
        }
        state.next_version = version;
        state.captured_bytes = next_bytes;
        state.changes.extend(captured);
        lease.complete();
        Ok(version)
    }

    pub(crate) async fn mark_continuity_lost(&self) {
        let mut state = self.state.lock().await;
        state.continuity_lost = true;
        state.baseline_required = true;
    }

    fn capture_checkpoint_from_state(
        &self,
        state: &InMemorySourceState,
    ) -> Result<CaptureCheckpoint> {
        let checkpoint = CaptureCheckpoint {
            migration_id: self.migration_id.clone(),
            source_identity: self.source_identity.clone(),
            capture_generation: state.capture_generation,
            next_version: state.next_version,
            captured_bytes: state.captured_bytes,
            max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            continuity_lost: state.continuity_lost,
            baseline_required: state.baseline_required,
        };
        checkpoint.validate()?;
        Ok(checkpoint)
    }

    pub(crate) async fn capture_checkpoint(&self) -> Result<CaptureCheckpoint> {
        let state = self.state.lock().await;
        self.capture_checkpoint_from_state(&state)
    }

    pub(crate) async fn source_write_authority(&self) -> Result<SourceWriteAuthority> {
        let state = self.state.lock().await;
        let authority = SourceWriteAuthority {
            generation: state.authority_generation,
            fenced: state.source_writes_fenced,
        };
        authority.validate()?;
        Ok(authority)
    }

    pub(crate) async fn fence_source_writes(
        &self,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<()> {
        if expected_generation == 0 || next_generation <= expected_generation {
            bail!("source authority generations must advance monotonically");
        }
        let mut state = self.state.lock().await;
        if state.source_writes_fenced {
            bail!("source writes are already durably fenced");
        }
        if state.authority_generation != expected_generation {
            bail!("source writer generation is stale");
        }
        state.authority_generation = next_generation;
        state.source_writes_fenced = true;
        Ok(())
    }

    pub(crate) async fn reopen_source_writes(&self, expected_generation: u64) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.authority_generation != expected_generation {
            bail!("source writer generation is stale");
        }
        state.source_writes_fenced = false;
        Ok(())
    }

    pub(crate) async fn row(&self, table_name: &str, stable_key: &str) -> Option<VersionedRow> {
        self.state
            .lock()
            .await
            .rows
            .get(table_name)
            .and_then(|rows| rows.get(stable_key).cloned())
    }

    pub(crate) async fn capture_watermark(&self) -> Result<i64> {
        <Self as MigrationSource>::capture_watermark(self).await
    }

    async fn read_changes(
        &self,
        after: i64,
        through: Option<i64>,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>> {
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        select_changes(&state.changes, after, through, limits)
    }

    pub(crate) async fn release_capture_through(&self, through: i64) -> Result<()> {
        if through < 0 {
            bail!("in-memory migration capture release watermark must not be negative");
        }
        let mut state = self.state.lock().await;
        state
            .changes
            .retain(|change| change.source_version > through);
        state.captured_bytes = state.changes.iter().try_fold(0_u64, |total, change| {
            total
                .checked_add(
                    change
                        .payload
                        .as_ref()
                        .map_or(0, |payload| payload.len() as u64),
                )
                .ok_or_else(|| anyhow!("source change capture byte count overflowed"))
        })?;
        Ok(())
    }
}

fn select_changes(
    changes: &[CapturedChange],
    after: i64,
    through: Option<i64>,
    limits: ChangeBatchLimits,
) -> Result<Vec<CapturedChange>> {
    if after < 0 || through.is_some_and(|value| value < after) {
        bail!("source change cursor and limit are invalid");
    }
    let candidates = changes
        .iter()
        .filter(|change| {
            change.source_version > after
                && through.is_none_or(|watermark| change.source_version <= watermark)
        })
        .cloned()
        .collect::<Vec<_>>();
    select_change_batch(&candidates, limits)
}

pub(crate) fn select_change_batch(
    changes: &[CapturedChange],
    limits: ChangeBatchLimits,
) -> Result<Vec<CapturedChange>> {
    limits.validate()?;
    let mut selected = Vec::new();
    let mut selected_bytes = 0_usize;
    let mut offset = 0_usize;
    while offset < changes.len() {
        let source_version = changes[offset].source_version;
        let group_start = offset;
        while offset < changes.len() && changes[offset].source_version == source_version {
            offset += 1;
        }
        let group = &changes[group_start..offset];
        if group.len() > limits.max_rows {
            bail!("source transaction exceeds the bounded change row budget");
        }
        let group_bytes = group.iter().try_fold(0_usize, |total, change| {
            total
                .checked_add(change.payload.as_ref().map_or(0, Vec::len))
                .ok_or_else(|| anyhow!("migration change byte count overflowed"))
        })?;
        if group_bytes > limits.max_bytes {
            bail!("source transaction exceeds the bounded change byte budget");
        }
        let next_rows = selected
            .len()
            .checked_add(group.len())
            .ok_or_else(|| anyhow!("migration change row count overflowed"))?;
        let next_bytes = selected_bytes
            .checked_add(group_bytes)
            .ok_or_else(|| anyhow!("migration change byte count overflowed"))?;
        if next_rows > limits.max_rows || next_bytes > limits.max_bytes {
            break;
        }
        selected.extend(group.iter().cloned());
        selected_bytes = next_bytes;
    }
    Ok(selected)
}

pub(crate) fn validate_contiguous_change_prefix(
    changes: &[CapturedChange],
    after: i64,
    through: i64,
) -> Result<()> {
    if after < 0 || through < after {
        bail!("source change prefix watermarks are invalid");
    }
    if after == through {
        if !changes.is_empty() {
            bail!("source returned changes for an empty watermark prefix");
        }
        return Ok(());
    }
    let first = changes
        .first()
        .ok_or_else(|| anyhow!("source change prefix is empty"))?;
    let expected_first = after
        .checked_add(1)
        .ok_or_else(|| anyhow!("source change watermark overflowed"))?;
    if first.source_version != expected_first {
        bail!("source change prefix does not start at the next watermark");
    }
    let mut previous_version = first.source_version;
    for change in changes.iter().skip(1) {
        if change.source_version < previous_version {
            bail!("source changes are not ordered by watermark");
        }
        if change.source_version > previous_version {
            let expected = previous_version
                .checked_add(1)
                .ok_or_else(|| anyhow!("source change watermark overflowed"))?;
            if change.source_version != expected {
                bail!("source change prefix contains a watermark gap");
            }
            previous_version = change.source_version;
        }
    }
    if previous_version > through {
        bail!("source change prefix exceeds its requested watermark");
    }
    Ok(())
}

#[async_trait]
impl MigrationSource for InMemoryMigrationSource {
    async fn capture_watermark(&self) -> Result<i64> {
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        Ok(state.next_version)
    }

    async fn capture_checkpoint(&self) -> Result<CaptureCheckpoint> {
        InMemoryMigrationSource::capture_checkpoint(self).await
    }

    async fn mark_baseline_complete(&self, capture_generation: u64, through: i64) -> Result<()> {
        InMemoryMigrationSource::mark_baseline_complete(self, capture_generation, through).await
    }

    async fn fence_source_writes(
        &self,
        expected_generation: u64,
        next_generation: u64,
    ) -> Result<()> {
        InMemoryMigrationSource::fence_source_writes(self, expected_generation, next_generation)
            .await
    }

    async fn reopen_source_writes(&self, expected_generation: u64) -> Result<()> {
        InMemoryMigrationSource::reopen_source_writes(self, expected_generation).await
    }

    async fn module_tables(&self, module: StorageModule) -> Result<Vec<String>> {
        Ok(self
            .state
            .lock()
            .await
            .module_tables
            .get(&module)
            .map(|tables| tables.iter().cloned().collect())
            .unwrap_or_default())
    }

    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)> {
        let state = self.state.lock().await;
        let Some(rows) = state.rows.get(table_name) else {
            return Ok((None, None));
        };
        Ok((
            rows.keys().next().cloned(),
            rows.keys().next_back().cloned(),
        ))
    }

    async fn scan_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage> {
        limits.validate()?;
        let candidates = self
            .state
            .lock()
            .await
            .rows
            .get(table_name)
            .map(|rows| {
                rows.values()
                    .filter(|row| {
                        cursor.is_none_or(|cursor| row.stable_key.as_str() > cursor)
                            && upper_bound.is_none_or(|upper| row.stable_key.as_str() <= upper)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        for row in candidates.iter().cloned() {
            if rows.len() >= limits.max_rows {
                break;
            }
            if rows.is_empty() && row.payload.len() > limits.max_bytes {
                bail!("in-memory migration row exceeds the copy byte budget");
            }
            let next_bytes = bytes
                .checked_add(row.payload.len())
                .ok_or_else(|| anyhow!("source page byte count overflowed"))?;
            if next_bytes > limits.max_bytes {
                break;
            }
            bytes = next_bytes;
            rows.push(row);
        }
        let complete = rows.len() == candidates.len();
        Ok(SourcePage {
            next_cursor: rows.last().map(|row| row.stable_key.clone()),
            rows,
            complete,
        })
    }

    async fn scan_range_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourceRangePage> {
        limits.validate()?;
        self.before_verification_read().await;
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        let mut complete = true;
        if let Some(source_rows) = state.rows.get(table_name) {
            for row in source_rows.values().filter(|row| {
                cursor.is_none_or(|cursor| row.stable_key.as_str() > cursor)
                    && lower_bound.is_none_or(|lower| row.stable_key.as_str() >= lower)
                    && upper_bound.is_none_or(|upper| row.stable_key.as_str() <= upper)
            }) {
                if rows.len() >= limits.max_rows {
                    complete = false;
                    break;
                }
                if rows.is_empty() && row.payload.len() > limits.max_bytes {
                    bail!("in-memory migration row exceeds the coverage byte budget");
                }
                let next_bytes = bytes
                    .checked_add(row.payload.len())
                    .ok_or_else(|| anyhow!("coverage page byte count overflowed"))?;
                if next_bytes > limits.max_bytes {
                    complete = false;
                    break;
                }
                bytes = next_bytes;
                rows.push(row.clone());
            }
        }
        Ok(SourceRangePage {
            source_watermark: state.next_version,
            page: SourcePage {
                next_cursor: rows.last().map(|row| row.stable_key.clone()),
                rows,
                complete,
            },
        })
    }

    async fn rows_in_range_snapshot(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<SourceRangeSnapshot> {
        self.before_verification_read().await;
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        Ok(SourceRangeSnapshot {
            source_watermark: state.next_version,
            rows: state
                .rows
                .get(table_name)
                .map(|rows| {
                    rows.range(range.first_key.clone()..=range.last_key.clone())
                        .map(|(_, row)| row.clone())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    async fn changes_after(
        &self,
        after: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>> {
        self.read_changes(after, None, limits).await
    }

    async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limits: ChangeBatchLimits,
    ) -> Result<Vec<CapturedChange>> {
        self.read_changes(after, Some(through), limits).await
    }

    async fn release_capture_through(&self, through: i64) -> Result<()> {
        InMemoryMigrationSource::release_capture_through(self, through).await
    }

    fn migration_id(&self) -> &str {
        &self.migration_id
    }

    fn source_identity(&self) -> &str {
        &self.source_identity
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InMemoryMigrationTarget {
    state: Arc<Mutex<InMemoryTargetState>>,
}

#[derive(Debug, Default)]
struct InMemoryTargetState {
    plans: BTreeMap<(String, String), TableCopyPlan>,
    checkpoints: BTreeMap<(String, String), EngineCheckpoint>,
    rows: BTreeMap<String, BTreeMap<String, VersionedRow>>,
    chunk_proofs: BTreeMap<(String, String, String, String), ChunkProof>,
    coverage_checkpoints: BTreeMap<(String, String), CoverageCheckpoint>,
    coverage_proofs: BTreeMap<(String, String), TableCoverageProof>,
    fail_next_operation: bool,
    fail_after_apply: bool,
}

impl InMemoryMigrationTarget {
    pub(crate) async fn row(&self, table_name: &str, stable_key: &str) -> Option<VersionedRow> {
        self.state
            .lock()
            .await
            .rows
            .get(table_name)
            .and_then(|rows| rows.get(stable_key).cloned())
    }

    pub(crate) async fn seed_row_for_test(
        &self,
        table_name: &str,
        stable_key: &str,
        source_version: i64,
        payload: &[u8],
    ) {
        self.state
            .lock()
            .await
            .rows
            .entry(table_name.to_owned())
            .or_default()
            .insert(
                stable_key.to_owned(),
                VersionedRow {
                    stable_key: stable_key.to_owned(),
                    source_version,
                    payload: payload.to_vec(),
                },
            );
    }

    pub(crate) async fn remove_row_for_test(&self, table_name: &str, stable_key: &str) {
        if let Some(rows) = self.state.lock().await.rows.get_mut(table_name) {
            rows.remove(stable_key);
        }
    }

    pub(crate) async fn fail_next_operation(&self) {
        self.state.lock().await.fail_next_operation = true;
    }

    pub(crate) async fn fail_after_apply_for_test(&self) {
        self.state.lock().await.fail_after_apply = true;
    }

    pub(crate) async fn remove_plan_for_test(&self, table_name: &str) {
        let mut state = self.state.lock().await;
        state.plans.retain(|(_, table), _| table != table_name);
        state
            .checkpoints
            .retain(|(_, table), _| table != table_name);
    }

    fn maybe_fail(state: &mut InMemoryTargetState) -> Result<()> {
        if state.fail_next_operation {
            state.fail_next_operation = false;
            bail!("in-memory migration target operation failed for retry test");
        }
        Ok(())
    }

    fn maybe_fail_after_apply(state: &mut InMemoryTargetState) -> Result<()> {
        if state.fail_after_apply {
            state.fail_after_apply = false;
            bail!("in-memory migration target acknowledgement failed after apply");
        }
        Ok(())
    }
}

#[async_trait]
impl MigrationTarget for InMemoryMigrationTarget {
    async fn known_tables(&self, migration_id: &str) -> Result<Vec<String>> {
        Ok(self
            .state
            .lock()
            .await
            .plans
            .iter()
            .filter(|((id, _), _)| id == migration_id)
            .map(|((_, table), _)| table.clone())
            .collect())
    }

    async fn load_plan(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCopyPlan>> {
        Ok(self
            .state
            .lock()
            .await
            .plans
            .get(&(migration_id.to_owned(), table_name.to_owned()))
            .cloned())
    }

    async fn save_plan(&self, plan: &TableCopyPlan) -> Result<()> {
        plan.validate()?;
        let mut state = self.state.lock().await;
        Self::maybe_fail(&mut state)?;
        let key = (plan.migration_id.clone(), plan.table_name.clone());
        if let Some(existing) = state.plans.get(&key)
            && existing != plan
        {
            bail!("migration plan conflicts with an existing target plan");
        }
        state
            .plans
            .entry(key.clone())
            .or_insert_with(|| plan.clone());
        state
            .checkpoints
            .entry(key)
            .or_insert_with(|| EngineCheckpoint {
                migration_id: plan.migration_id.clone(),
                table_name: plan.table_name.clone(),
                installation_id: plan.installation_id.clone(),
                source_identity: plan.source_identity.clone(),
                target_identity: plan.target_identity.clone(),
                cursor: None,
                scan_upper_bound: plan.scan_upper_bound.clone(),
                capture_start: plan.capture_start,
                captured_through: plan.capture_start,
                applied_through: plan.capture_start,
                copied_rows: 0,
                copied_bytes: 0,
                complete: false,
            });
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<EngineCheckpoint>> {
        Ok(self
            .state
            .lock()
            .await
            .checkpoints
            .get(&(migration_id.to_owned(), table_name.to_owned()))
            .cloned())
    }

    async fn apply_baseline_page(
        &self,
        plan: &TableCopyPlan,
        page: &SourcePage,
        checkpoint: &EngineCheckpoint,
    ) -> Result<()> {
        checkpoint.validate()?;
        let cursor = self
            .load_checkpoint(&plan.migration_id, &plan.table_name)
            .await?
            .and_then(|checkpoint| checkpoint.cursor);
        page.validate(
            &plan.table_name,
            cursor.as_deref(),
            plan.scan_upper_bound.as_deref(),
            CopyLimits::default(),
        )?;
        if checkpoint.migration_id != plan.migration_id
            || checkpoint.table_name != plan.table_name
            || checkpoint.installation_id != plan.installation_id
            || checkpoint.source_identity != plan.source_identity
            || checkpoint.target_identity != plan.target_identity
        {
            bail!("baseline checkpoint does not match its plan");
        }
        let mut state = self.state.lock().await;
        Self::maybe_fail(&mut state)?;
        let key = (plan.migration_id.clone(), plan.table_name.clone());
        let existing = state
            .checkpoints
            .get(&key)
            .ok_or_else(|| anyhow!("migration checkpoint is missing"))?
            .clone();
        validate_checkpoint_progress(Some(&existing), checkpoint)?;
        {
            let rows = state.rows.entry(plan.table_name.clone()).or_default();
            for row in &page.rows {
                upsert_versioned_row(rows, row);
            }
        }
        state.checkpoints.insert(key, checkpoint.clone());
        Ok(())
    }

    async fn apply_change_batch(
        &self,
        migration_id: &str,
        source_identity: &str,
        changes: &[CapturedChange],
        checkpoint: &EngineCheckpoint,
    ) -> Result<()> {
        checkpoint.validate()?;
        if checkpoint.migration_id != migration_id || checkpoint.source_identity != source_identity
        {
            bail!("replay checkpoint does not match its migration");
        }
        validate_persisted_identity(
            "replay checkpoint",
            &checkpoint.migration_id,
            &checkpoint.installation_id,
            &checkpoint.source_identity,
            &checkpoint.target_identity,
        )?;
        let mut state = self.state.lock().await;
        Self::maybe_fail(&mut state)?;
        let key = (migration_id.to_owned(), checkpoint.table_name.clone());
        let existing = state
            .checkpoints
            .get(&key)
            .ok_or_else(|| anyhow!("migration checkpoint is missing"))?
            .clone();
        validate_checkpoint_progress(Some(&existing), checkpoint)?;
        {
            let rows = state.rows.entry(checkpoint.table_name.clone()).or_default();
            for change in changes {
                change.validate_for(migration_id, source_identity)?;
                if change.table_name != checkpoint.table_name
                    || change.source_version > checkpoint.captured_through
                {
                    bail!("replay change does not match its checkpoint");
                }
                apply_change_to_rows(rows, change);
            }
        }
        state.checkpoints.insert(key, checkpoint.clone());
        Self::maybe_fail_after_apply(&mut state)?;
        Ok(())
    }

    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>> {
        Ok(self
            .state
            .lock()
            .await
            .rows
            .get(table_name)
            .map(|rows| {
                rows.range(range.first_key.clone()..=range.last_key.clone())
                    .map(|(_, row)| row.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn scan_range_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        lower_bound: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage> {
        limits.validate()?;
        let state = self.state.lock().await;
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        let mut complete = true;
        if let Some(target_rows) = state.rows.get(table_name) {
            for row in target_rows.values().filter(|row| {
                cursor.is_none_or(|cursor| row.stable_key.as_str() > cursor)
                    && lower_bound.is_none_or(|lower| row.stable_key.as_str() >= lower)
                    && upper_bound.is_none_or(|upper| row.stable_key.as_str() <= upper)
            }) {
                if rows.len() >= limits.max_rows {
                    complete = false;
                    break;
                }
                if rows.is_empty() && row.payload.len() > limits.max_bytes {
                    bail!("in-memory migration row exceeds the coverage byte budget");
                }
                let next_bytes = bytes
                    .checked_add(row.payload.len())
                    .ok_or_else(|| anyhow!("coverage page byte count overflowed"))?;
                if next_bytes > limits.max_bytes {
                    complete = false;
                    break;
                }
                bytes = next_bytes;
                rows.push(row.clone());
            }
        }
        Ok(SourcePage {
            next_cursor: rows.last().map(|row| row.stable_key.clone()),
            rows,
            complete,
        })
    }

    async fn table_bounds(&self, table_name: &str) -> Result<(Option<String>, Option<String>)> {
        let state = self.state.lock().await;
        let Some(rows) = state.rows.get(table_name) else {
            return Ok((None, None));
        };
        Ok((
            rows.keys().next().cloned(),
            rows.keys().next_back().cloned(),
        ))
    }

    async fn save_chunk_proof(&self, proof: &ChunkProof) -> Result<()> {
        proof.validate()?;
        let mut state = self.state.lock().await;
        Self::maybe_fail(&mut state)?;
        let key = (
            proof.migration_id.clone(),
            proof.table_name.clone(),
            proof.range.first_key.clone(),
            proof.range.last_key.clone(),
        );
        if let Some(existing) = state.chunk_proofs.get(&key)
            && existing != proof
        {
            bail!("migration chunk proof conflicts with an existing identity");
        }
        state.chunk_proofs.insert(key, proof.clone());
        Ok(())
    }

    async fn verified_chunks(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Vec<ChunkProof>> {
        Ok(self
            .state
            .lock()
            .await
            .chunk_proofs
            .iter()
            .filter(|((id, table, _, _), _)| id == migration_id && table == table_name)
            .map(|(_, proof)| proof.clone())
            .collect())
    }

    async fn save_coverage_checkpoint(&self, checkpoint: &CoverageCheckpoint) -> Result<()> {
        checkpoint.validate()?;
        let mut state = self.state.lock().await;
        Self::maybe_fail(&mut state)?;
        let key = (
            checkpoint.migration_id.clone(),
            checkpoint.table_name.clone(),
        );
        if let Some(existing) = state.coverage_checkpoints.get(&key)
            && (existing.installation_id != checkpoint.installation_id
                || existing.source_identity != checkpoint.source_identity
                || existing.target_identity != checkpoint.target_identity)
        {
            bail!("migration coverage checkpoint belongs to another identity");
        }
        state.coverage_checkpoints.insert(key, checkpoint.clone());
        Ok(())
    }

    async fn load_coverage_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<CoverageCheckpoint>> {
        Ok(self
            .state
            .lock()
            .await
            .coverage_checkpoints
            .get(&(migration_id.to_owned(), table_name.to_owned()))
            .cloned())
    }

    async fn save_table_coverage_proof(&self, proof: &TableCoverageProof) -> Result<()> {
        proof.validate()?;
        let mut state = self.state.lock().await;
        Self::maybe_fail(&mut state)?;
        let key = (proof.migration_id.clone(), proof.table_name.clone());
        if let Some(existing) = state.coverage_proofs.get(&key)
            && (existing.installation_id != proof.installation_id
                || existing.source_identity != proof.source_identity
                || existing.target_identity != proof.target_identity)
        {
            bail!("migration coverage proof belongs to another identity");
        }
        state.coverage_proofs.insert(key, proof.clone());
        Ok(())
    }

    async fn load_table_coverage_proof(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<TableCoverageProof>> {
        Ok(self
            .state
            .lock()
            .await
            .coverage_proofs
            .get(&(migration_id.to_owned(), table_name.to_owned()))
            .cloned())
    }
}

fn upsert_versioned_row(rows: &mut BTreeMap<String, VersionedRow>, row: &VersionedRow) {
    let should_replace = rows
        .get(&row.stable_key)
        .is_none_or(|existing| existing.source_version <= row.source_version);
    if should_replace {
        rows.insert(row.stable_key.clone(), row.clone());
    }
}

fn apply_change_to_rows(rows: &mut BTreeMap<String, VersionedRow>, change: &CapturedChange) {
    match change.kind {
        ChangeKind::Insert | ChangeKind::Update => upsert_versioned_row(
            rows,
            &VersionedRow {
                stable_key: change.stable_key.clone(),
                source_version: change.source_version,
                payload: change.payload.clone().unwrap_or_default(),
            },
        ),
        ChangeKind::Delete => {
            if rows
                .get(&change.stable_key)
                .is_none_or(|row| row.source_version <= change.source_version)
            {
                rows.remove(&change.stable_key);
            }
        }
        ChangeKind::Replace => {
            if let Some(previous_key) = &change.previous_key
                && rows
                    .get(previous_key)
                    .is_none_or(|row| row.source_version <= change.source_version)
            {
                rows.remove(previous_key);
            }
            upsert_versioned_row(
                rows,
                &VersionedRow {
                    stable_key: change.stable_key.clone(),
                    source_version: change.source_version,
                    payload: change.payload.clone().unwrap_or_default(),
                },
            );
        }
    }
}

pub(crate) fn validate_checkpoint_progress(
    existing: Option<&EngineCheckpoint>,
    checkpoint: &EngineCheckpoint,
) -> Result<()> {
    if checkpoint.captured_through < checkpoint.applied_through {
        bail!("migration checkpoint applies past its captured watermark");
    }
    if let Some(existing) = existing
        && (checkpoint.captured_through < existing.captured_through
            || checkpoint.applied_through < existing.applied_through
            || checkpoint.copied_rows < existing.copied_rows
            || checkpoint.copied_bytes < existing.copied_bytes
            || (existing.complete && !checkpoint.complete))
    {
        bail!("migration checkpoint regresses durable progress");
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct FileMigrationStore {
    path: PathBuf,
    context: Arc<Mutex<AuthorityRecoveryContext>>,
    lock: Arc<Mutex<()>>,
    fail_next_publication: Arc<AtomicBool>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredMigrationState {
    manifest: Option<StoredManifest>,
    #[serde(default)]
    recovery_context: Option<StoredRecoveryContext>,
    statuses: Vec<StoredStatus>,
    checkpoints: Vec<StoredCheckpoint>,
    outbox: Vec<StoredOutboxItem>,
    reservations: Vec<StoredReservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredRecoveryContext {
    migration_id: String,
    installation_id: String,
    source_identity: String,
    module: String,
    source_watermark: i64,
    target_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredManifest {
    migration_id: String,
    installation_id: String,
    source_identity: String,
    module: String,
    backend: String,
    generation: u64,
    source_watermark: i64,
    target_identity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredStatus {
    migration_id: String,
    installation_id: String,
    module: String,
    state: String,
    source_identity: String,
    target_identity: String,
    authority_generation: u64,
    capacity_bytes: u64,
    held_bytes: u64,
    hold_reason: Option<String>,
    updated_at_epoch_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCheckpoint {
    migration_id: String,
    table_name: String,
    stable_key: String,
    cursor: Option<String>,
    captured_through: Option<i64>,
    applied_through: Option<i64>,
    #[serde(default)]
    installation_id: String,
    source_identity: String,
    #[serde(default)]
    target_identity: String,
    authority_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredOutboxItem {
    outbox_id: String,
    migration_id: String,
    source_identity: String,
    source_version: i64,
    module: String,
    authority_generation: u64,
    payload_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredReservation {
    reservation_id: String,
    migration_id: String,
    source_identity: String,
    module: String,
    authority_generation: u64,
    first_stable_id: String,
    last_stable_id: String,
    reserved_bytes: u64,
}

impl FileMigrationStore {
    pub(crate) fn new(path: impl Into<PathBuf>, context: AuthorityRecoveryContext) -> Self {
        Self {
            path: path.into(),
            context: Arc::new(Mutex::new(context)),
            lock: Arc::new(Mutex::new(())),
            fail_next_publication: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn fail_next_authority_publication(&self) {
        self.fail_next_publication.store(true, Ordering::Release);
    }

    pub(crate) async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord> {
        <Self as AuthorityManifestStore>::load_authority_manifest(self, module).await
    }

    pub(crate) async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
        expected: &AuthorityRecoveryContext,
    ) -> Result<AuthorityPublication> {
        <Self as AuthorityManifestStore>::publish_authority_manifest(self, manifest, expected).await
    }

    async fn read_state(&self) -> Result<StoredMigrationState> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parse migration metadata {}", self.path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(error) => Err(error)
                .with_context(|| format!("read migration metadata {}", self.path.display())),
        }
    }

    pub(crate) async fn set_recovery_context(
        &self,
        context: &AuthorityRecoveryContext,
    ) -> Result<()> {
        if context.migration_id.trim().is_empty()
            || context.installation_id.trim().is_empty()
            || context.source_identity.trim().is_empty()
            || context.target_identity.trim().is_empty()
            || context.source_watermark < 0
        {
            bail!("migration recovery context is invalid");
        }
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        let stored_context = value_to_stored_recovery_context(context);
        if let Some(existing) = state.recovery_context.as_ref()
            && existing != &stored_context
        {
            bail!("migration recovery context conflicts with the durable marker");
        }
        if let Some(existing) = state.manifest.as_ref() {
            let existing = stored_manifest_to_value(existing.clone())?;
            if !manifest_matches_recovery_context(&existing, context) {
                bail!("migration recovery context conflicts with the durable authority manifest");
            }
            state.recovery_context = None;
        } else {
            state.recovery_context = Some(stored_context);
        }
        self.write_state(&state).await?;
        *self.context.lock().await = context.clone();
        Ok(())
    }

    async fn write_state(&self, state: &StoredMigrationState) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(state).context("serialize migration metadata")?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("create migration metadata directory {}", parent.display()))?;
        let temporary = self.path.with_extension("tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .with_context(|| {
                format!("open temporary migration metadata {}", temporary.display())
            })?;
        file.write_all(&bytes).context("write migration metadata")?;
        file.sync_all().context("sync migration metadata")?;
        fs::rename(&temporary, &self.path).with_context(|| {
            format!(
                "replace migration metadata {} with {}",
                self.path.display(),
                temporary.display()
            )
        })?;
        File::open(parent)
            .with_context(|| format!("open migration metadata directory {}", parent.display()))?
            .sync_all()
            .with_context(|| format!("sync migration metadata directory {}", parent.display()))?;
        Ok(())
    }
}

#[async_trait]
impl AuthorityManifestStore for FileMigrationStore {
    async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord> {
        let _guard = self.lock.lock().await;
        let state = self.read_state().await?;
        let Some(stored) = state.manifest else {
            let context = if let Some(stored) = state.recovery_context {
                let context = stored_recovery_context_to_value(stored)?;
                if context.module != module {
                    return Ok(AuthorityRecoveryRecord::Indeterminate);
                }
                context
            } else {
                self.context.lock().await.clone()
            };
            if context.module != module {
                return Ok(AuthorityRecoveryRecord::Indeterminate);
            }
            return Ok(AuthorityRecoveryRecord::NeverMigrated {
                migration_id: context.migration_id,
                installation_id: context.installation_id,
                module,
                source_identity: context.source_identity,
                target_identity: context.target_identity,
                source_watermark: context.source_watermark,
            });
        };
        let manifest = stored_manifest_to_value(stored)?;
        if manifest.module != module {
            return Ok(AuthorityRecoveryRecord::Indeterminate);
        }
        Ok(AuthorityRecoveryRecord::Manifest(manifest))
    }

    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
        expected: &AuthorityRecoveryContext,
    ) -> Result<AuthorityPublication> {
        if self.fail_next_publication.swap(false, Ordering::AcqRel) {
            bail!("authority publication failed for retry test");
        }
        if manifest.module != expected.module
            || manifest.migration_id != expected.migration_id
            || manifest.installation_id != expected.installation_id
            || manifest.source_identity != expected.source_identity
            || manifest.target_identity != expected.target_identity
            || manifest.source_watermark != expected.source_watermark
            || manifest.authority.backend != BackendKind::Postgres
        {
            bail!("authority publication identity does not match its recovery context");
        }
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        if let Some(existing) = state.manifest.as_ref() {
            let existing = stored_manifest_to_value(existing.clone())?;
            if existing == *manifest {
                if state.recovery_context.take().is_some() {
                    self.write_state(&state).await?;
                }
                return Ok(AuthorityPublication::Committed);
            }
            return Ok(AuthorityPublication::Conflict);
        }
        let Some(stored_context) = state.recovery_context.as_ref() else {
            return Ok(AuthorityPublication::Conflict);
        };
        let durable_context = stored_recovery_context_to_value(stored_context.clone())?;
        if durable_context != *expected {
            return Ok(AuthorityPublication::Conflict);
        }
        state.manifest = Some(value_to_stored_manifest(manifest));
        state.recovery_context = None;
        self.write_state(&state).await?;
        Ok(AuthorityPublication::Committed)
    }
}

#[async_trait]
impl MigrationMetadataStore for FileMigrationStore {
    async fn set_recovery_context(&self, context: &AuthorityRecoveryContext) -> Result<()> {
        FileMigrationStore::set_recovery_context(self, context).await
    }

    async fn load_status(&self, module: StorageModule) -> Result<Option<MigrationStatus>> {
        let _guard = self.lock.lock().await;
        let state = self.read_state().await?;
        state
            .statuses
            .into_iter()
            .find(|status| parse_module(&status.module).ok() == Some(module))
            .map(stored_status_to_value)
            .transpose()
    }

    async fn save_status(&self, status: &MigrationStatus) -> Result<()> {
        status.validate()?;
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        let module = module_name(status.module).to_owned();
        state
            .statuses
            .retain(|stored| stored.migration_id != status.migration_id || stored.module != module);
        state.statuses.push(value_to_stored_status(status));
        self.write_state(&state).await
    }

    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<MigrationCheckpoint>> {
        let _guard = self.lock.lock().await;
        let state = self.read_state().await?;
        let identity_context = if let Some(stored) = state.recovery_context.clone() {
            stored_recovery_context_to_value(stored)?
        } else {
            self.context.lock().await.clone()
        };
        state
            .checkpoints
            .into_iter()
            .find(|checkpoint| {
                checkpoint.migration_id == migration_id && checkpoint.table_name == table_name
            })
            .map(|stored| {
                let checkpoint = stored_checkpoint_to_value(stored)?;
                validate_checkpoint_identity_against_context(&checkpoint, &identity_context)?;
                Ok(checkpoint)
            })
            .transpose()
    }

    async fn save_checkpoint(&self, checkpoint: &MigrationCheckpoint) -> Result<()> {
        checkpoint.validate()?;
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        let identity_context = if let Some(stored) = state.recovery_context.clone() {
            stored_recovery_context_to_value(stored)?
        } else {
            self.context.lock().await.clone()
        };
        validate_checkpoint_identity_against_context(checkpoint, &identity_context)?;
        state.checkpoints.retain(|stored| {
            stored.migration_id != checkpoint.migration_id
                || stored.table_name != checkpoint.table_name
        });
        state
            .checkpoints
            .push(value_to_stored_checkpoint(checkpoint));
        self.write_state(&state).await
    }

    async fn enqueue_outbox(&self, item: &MigrationOutboxItem) -> Result<bool> {
        item.validate()?;
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        let incoming = value_to_stored_outbox(item);
        if let Some(existing) = state
            .outbox
            .iter()
            .find(|stored| stored.outbox_id == item.outbox_id)
        {
            if existing == &incoming {
                return Ok(false);
            }
            bail!("migration outbox id conflicts with an existing payload or identity");
        }
        state.outbox.push(incoming);
        self.write_state(&state).await?;
        Ok(true)
    }

    async fn acknowledge_outbox(&self, outbox_id: &str, applied_through: i64) -> Result<()> {
        if applied_through < 0 {
            bail!("acknowledged migration watermark must not be negative");
        }
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        let Some(item) = state.outbox.iter().find(|item| item.outbox_id == outbox_id) else {
            bail!("migration outbox item is missing");
        };
        if applied_through < item.source_version {
            bail!("migration outbox acknowledgement regresses its source version");
        }
        state.outbox.retain(|item| item.outbox_id != outbox_id);
        self.write_state(&state).await
    }

    async fn reserve_capacity(&self, reservation: &MigrationReservation) -> Result<bool> {
        reservation.validate()?;
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        if state
            .reservations
            .iter()
            .any(|stored| stored.reservation_id == reservation.reservation_id)
        {
            return Ok(false);
        }
        state
            .reservations
            .push(value_to_stored_reservation(reservation));
        self.write_state(&state).await?;
        Ok(true)
    }

    async fn release_capacity(&self, reservation_id: &str) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.read_state().await?;
        state
            .reservations
            .retain(|reservation| reservation.reservation_id != reservation_id);
        self.write_state(&state).await
    }
}

fn module_name(module: StorageModule) -> &'static str {
    match module {
        StorageModule::TrafficArchive => "traffic_archive",
        StorageModule::Analytics => "analytics",
        StorageModule::AccountRouting => "account_routing",
        StorageModule::TaskOperations => "task_operations",
        StorageModule::Performance => "performance",
    }
}

fn parse_module(value: &str) -> Result<StorageModule> {
    match value {
        "traffic_archive" => Ok(StorageModule::TrafficArchive),
        "analytics" => Ok(StorageModule::Analytics),
        "account_routing" => Ok(StorageModule::AccountRouting),
        "task_operations" => Ok(StorageModule::TaskOperations),
        "performance" => Ok(StorageModule::Performance),
        _ => bail!("unknown migration module: {value}"),
    }
}

fn backend_name(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Sqlite => "sqlite",
        BackendKind::Postgres => "postgres",
    }
}

fn parse_backend(value: &str) -> Result<BackendKind> {
    match value {
        "sqlite" => Ok(BackendKind::Sqlite),
        "postgres" => Ok(BackendKind::Postgres),
        _ => bail!("unknown migration backend: {value}"),
    }
}

fn state_name(state: MigrationState) -> &'static str {
    match state {
        MigrationState::Pending => "pending",
        MigrationState::Preparing => "preparing",
        MigrationState::Copying => "copying",
        MigrationState::CatchingUp => "catching_up",
        MigrationState::Verifying => "verifying",
        MigrationState::Ready => "ready",
        MigrationState::Switching => "switching",
        MigrationState::Postgres => "postgres",
        MigrationState::Paused => "paused",
        MigrationState::Blocked => "blocked",
        MigrationState::Recovering => "recovering",
        MigrationState::Failed => "failed",
    }
}

fn parse_state(value: &str) -> Result<MigrationState> {
    match value {
        "pending" => Ok(MigrationState::Pending),
        "preparing" => Ok(MigrationState::Preparing),
        "copying" => Ok(MigrationState::Copying),
        "catching_up" => Ok(MigrationState::CatchingUp),
        "verifying" => Ok(MigrationState::Verifying),
        "ready" => Ok(MigrationState::Ready),
        "switching" => Ok(MigrationState::Switching),
        "postgres" => Ok(MigrationState::Postgres),
        "paused" => Ok(MigrationState::Paused),
        "blocked" => Ok(MigrationState::Blocked),
        "recovering" => Ok(MigrationState::Recovering),
        "failed" => Ok(MigrationState::Failed),
        _ => bail!("unknown migration state: {value}"),
    }
}

fn value_to_stored_manifest(manifest: &ModuleAuthorityManifest) -> StoredManifest {
    StoredManifest {
        migration_id: manifest.migration_id.clone(),
        installation_id: manifest.installation_id.clone(),
        source_identity: manifest.source_identity.clone(),
        module: module_name(manifest.module).to_owned(),
        backend: backend_name(manifest.authority.backend).to_owned(),
        generation: manifest.authority.generation,
        source_watermark: manifest.source_watermark,
        target_identity: manifest.target_identity.clone(),
    }
}

fn value_to_stored_recovery_context(context: &AuthorityRecoveryContext) -> StoredRecoveryContext {
    StoredRecoveryContext {
        migration_id: context.migration_id.clone(),
        installation_id: context.installation_id.clone(),
        source_identity: context.source_identity.clone(),
        module: module_name(context.module).to_owned(),
        source_watermark: context.source_watermark,
        target_identity: context.target_identity.clone(),
    }
}

fn stored_recovery_context_to_value(
    stored: StoredRecoveryContext,
) -> Result<AuthorityRecoveryContext> {
    Ok(AuthorityRecoveryContext {
        migration_id: stored.migration_id,
        installation_id: stored.installation_id,
        module: parse_module(&stored.module)?,
        source_identity: stored.source_identity,
        target_identity: stored.target_identity,
        source_watermark: stored.source_watermark,
    })
}

fn stored_manifest_to_value(stored: StoredManifest) -> Result<ModuleAuthorityManifest> {
    Ok(ModuleAuthorityManifest {
        migration_id: stored.migration_id,
        installation_id: stored.installation_id,
        source_identity: stored.source_identity,
        module: parse_module(&stored.module)?,
        authority: ModuleAuthority {
            backend: parse_backend(&stored.backend)?,
            generation: stored.generation,
        },
        source_watermark: stored.source_watermark,
        target_identity: stored.target_identity,
    })
}

fn manifest_matches_recovery_context(
    manifest: &ModuleAuthorityManifest,
    context: &AuthorityRecoveryContext,
) -> bool {
    manifest.migration_id == context.migration_id
        && manifest.installation_id == context.installation_id
        && manifest.source_identity == context.source_identity
        && manifest.module == context.module
        && manifest.source_watermark == context.source_watermark
        && manifest.target_identity == context.target_identity
}

fn validate_checkpoint_identity_against_context(
    checkpoint: &MigrationCheckpoint,
    context: &AuthorityRecoveryContext,
) -> Result<()> {
    if checkpoint.migration_id != context.migration_id
        || checkpoint.installation_id != context.installation_id
        || checkpoint.source_identity != context.source_identity
        || checkpoint.target_identity != context.target_identity
    {
        bail!("migration checkpoint belongs to another durable migration identity");
    }
    Ok(())
}

fn value_to_stored_status(status: &MigrationStatus) -> StoredStatus {
    StoredStatus {
        migration_id: status.migration_id.clone(),
        installation_id: status.installation_id.clone(),
        module: module_name(status.module).to_owned(),
        state: state_name(status.state).to_owned(),
        source_identity: status.source_identity.clone(),
        target_identity: status.target_identity.clone(),
        authority_generation: status.authority_generation,
        capacity_bytes: status.capacity_bytes,
        held_bytes: status.held_bytes,
        hold_reason: status.hold_reason.clone(),
        updated_at_epoch_ms: status.updated_at_epoch_ms,
    }
}

fn stored_status_to_value(stored: StoredStatus) -> Result<MigrationStatus> {
    Ok(MigrationStatus {
        migration_id: stored.migration_id,
        installation_id: stored.installation_id,
        module: parse_module(&stored.module)?,
        state: parse_state(&stored.state)?,
        source_identity: stored.source_identity,
        target_identity: stored.target_identity,
        authority_generation: stored.authority_generation,
        capacity_bytes: stored.capacity_bytes,
        held_bytes: stored.held_bytes,
        hold_reason: stored.hold_reason,
        updated_at_epoch_ms: stored.updated_at_epoch_ms,
    })
}

fn value_to_stored_checkpoint(checkpoint: &MigrationCheckpoint) -> StoredCheckpoint {
    StoredCheckpoint {
        migration_id: checkpoint.migration_id.clone(),
        table_name: checkpoint.table_name.clone(),
        stable_key: checkpoint.stable_key.clone(),
        cursor: checkpoint.cursor.clone(),
        captured_through: checkpoint.captured_through,
        applied_through: checkpoint.applied_through,
        installation_id: checkpoint.installation_id.clone(),
        source_identity: checkpoint.source_identity.clone(),
        target_identity: checkpoint.target_identity.clone(),
        authority_generation: checkpoint.authority_generation,
    }
}

fn stored_checkpoint_to_value(stored: StoredCheckpoint) -> Result<MigrationCheckpoint> {
    let checkpoint = MigrationCheckpoint {
        migration_id: stored.migration_id,
        table_name: stored.table_name,
        stable_key: stored.stable_key,
        cursor: stored.cursor,
        captured_through: stored.captured_through,
        applied_through: stored.applied_through,
        installation_id: stored.installation_id,
        source_identity: stored.source_identity,
        target_identity: stored.target_identity,
        authority_generation: stored.authority_generation,
    };
    checkpoint.validate()?;
    Ok(checkpoint)
}

fn value_to_stored_outbox(item: &MigrationOutboxItem) -> StoredOutboxItem {
    StoredOutboxItem {
        outbox_id: item.outbox_id.clone(),
        migration_id: item.migration_id.clone(),
        source_identity: item.source_identity.clone(),
        source_version: item.source_version,
        module: module_name(item.module).to_owned(),
        authority_generation: item.authority_generation,
        payload_digest: item.payload_digest.clone(),
    }
}

fn value_to_stored_reservation(reservation: &MigrationReservation) -> StoredReservation {
    StoredReservation {
        reservation_id: reservation.reservation_id.clone(),
        migration_id: reservation.migration_id.clone(),
        source_identity: reservation.source_identity.clone(),
        module: module_name(reservation.module).to_owned(),
        authority_generation: reservation.authority_generation,
        first_stable_id: reservation.first_stable_id.clone(),
        last_stable_id: reservation.last_stable_id.clone(),
        reserved_bytes: reservation.reserved_bytes,
    }
}

#[derive(Debug)]
pub(crate) struct MigrationEngine<S, T, M> {
    source: Arc<S>,
    target: Arc<T>,
    store: Arc<M>,
    module: StorageModule,
    installation_id: String,
    target_identity: String,
    authority_generation: u64,
    limits: MigrationLimits,
    fence: SourceWriteFence,
    switching: Arc<Mutex<()>>,
    run_control: MigrationRunControl,
    cancellation: CancellationToken,
}

impl<S, T, M> Clone for MigrationEngine<S, T, M> {
    fn clone(&self) -> Self {
        Self {
            source: Arc::clone(&self.source),
            target: Arc::clone(&self.target),
            store: Arc::clone(&self.store),
            module: self.module,
            installation_id: self.installation_id.clone(),
            target_identity: self.target_identity.clone(),
            authority_generation: self.authority_generation,
            limits: self.limits,
            fence: self.fence.clone(),
            switching: Arc::clone(&self.switching),
            run_control: self.run_control.clone(),
            cancellation: self.cancellation.clone(),
        }
    }
}

impl<S, T, M> MigrationEngine<S, T, M>
where
    S: MigrationSource + 'static,
    T: MigrationTarget + 'static,
    M: MigrationMetadataStore + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        source: Arc<S>,
        target: Arc<T>,
        store: Arc<M>,
        module: StorageModule,
        installation_id: impl Into<String>,
        target_identity: impl Into<String>,
        authority_generation: u64,
        limits: MigrationLimits,
        fence: SourceWriteFence,
    ) -> Result<Self> {
        let installation_id = installation_id.into();
        let target_identity = target_identity.into();
        if source.migration_id().trim().is_empty()
            || source.source_identity().trim().is_empty()
            || installation_id.trim().is_empty()
            || target_identity.trim().is_empty()
            || authority_generation == 0
        {
            bail!("migration engine requires complete identities and generation");
        }
        limits.validate()?;
        Ok(Self {
            source,
            target,
            store,
            module,
            installation_id,
            target_identity,
            authority_generation,
            limits,
            fence,
            switching: Arc::new(Mutex::new(())),
            run_control: MigrationRunControl::default(),
            cancellation: CancellationToken::new(),
        })
    }

    pub(crate) fn run_control(&self) -> MigrationRunControl {
        self.run_control.clone()
    }

    pub(crate) fn with_cancellation_token(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    async fn cancellable<F, R, E>(&self, operation: F) -> Result<R>
    where
        F: Future<Output = std::result::Result<R, E>>,
        E: Into<anyhow::Error>,
    {
        tokio::select! {
            _ = self.cancellation.cancelled() => {
                Err(anyhow!("storage migration engine cancelled"))
            }
            result = operation => result.map_err(Into::into),
        }
    }

    async fn ensure_capture_ready(&self) -> Result<CaptureCheckpoint> {
        let checkpoint = self.source.capture_checkpoint().await?;
        if checkpoint.continuity_lost || checkpoint.baseline_required {
            bail!(
                "migration capture is not baseline-complete; complete a fresh source baseline before continuing"
            );
        }
        Ok(checkpoint)
    }

    pub(crate) async fn mark_baseline_complete(&self, through: i64) -> Result<()> {
        self.ensure_running().await?;
        let checkpoint = self.source.capture_checkpoint().await?;
        if checkpoint.continuity_lost {
            bail!("migration capture continuity has been lost; start a new capture generation");
        }
        self.source
            .mark_baseline_complete(checkpoint.capture_generation, through)
            .await
    }

    fn validate_plan_identity(&self, plan: &TableCopyPlan) -> Result<()> {
        if plan.migration_id != self.source.migration_id()
            || plan.installation_id != self.installation_id
            || plan.source_identity != self.source.source_identity()
            || plan.target_identity != self.target_identity
        {
            bail!("durable migration plan belongs to another migration identity");
        }
        Ok(())
    }

    fn validate_checkpoint_identity(&self, checkpoint: &EngineCheckpoint) -> Result<()> {
        if checkpoint.migration_id != self.source.migration_id()
            || checkpoint.installation_id != self.installation_id
            || checkpoint.source_identity != self.source.source_identity()
            || checkpoint.target_identity != self.target_identity
        {
            bail!("durable migration checkpoint belongs to another migration identity");
        }
        Ok(())
    }

    fn validate_coverage_identity(
        &self,
        migration_id: &str,
        installation_id: &str,
        source_identity: &str,
        target_identity: &str,
    ) -> Result<()> {
        if migration_id != self.source.migration_id()
            || installation_id != self.installation_id
            || source_identity != self.source.source_identity()
            || target_identity != self.target_identity
        {
            bail!("durable migration coverage state belongs to another migration identity");
        }
        Ok(())
    }

    pub(crate) async fn prepare(&self, table_names: &[String]) -> Result<()> {
        self.ensure_running().await?;
        if table_names.is_empty() {
            bail!("migration preparation requires at least one table");
        }
        self.ensure_capture_ready().await?;
        let source_table_set = self
            .source
            .module_tables(self.module)
            .await?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let requested_table_set = table_names.iter().cloned().collect::<BTreeSet<_>>();
        if requested_table_set.len() != table_names.len() || requested_table_set != source_table_set
        {
            bail!("migration preparation must cover the selected module's exact table set");
        }
        let mut existing_plans = BTreeMap::new();
        for table_name in table_names {
            if let Some(plan) = self
                .target
                .load_plan(self.source.migration_id(), table_name)
                .await?
            {
                if plan.migration_id != self.source.migration_id()
                    || plan.table_name != *table_name
                    || plan.capture_start < 0
                {
                    bail!("durable migration plan has an invalid identity");
                }
                self.validate_plan_identity(&plan)?;
                validate_table_bounds(
                    "durable migration plan",
                    &plan.scan_lower_bound,
                    &plan.scan_upper_bound,
                )?;
                existing_plans.insert(table_name.clone(), plan);
            }
        }
        // A durable plan's capture_start is the migration generation. Reuse it for every table
        // after a partial prepare so a restart never creates a second capture boundary.
        let capture_start = if let Some(plan) = existing_plans.values().next() {
            let capture_start = plan.capture_start;
            if existing_plans
                .values()
                .any(|existing| existing.capture_start != capture_start)
            {
                bail!("durable migration plans use different capture generations");
            }
            capture_start
        } else {
            self.source.capture_watermark().await?
        };
        for table_name in table_names {
            let existing = existing_plans.get(table_name);
            let plan = if let Some(plan) = existing {
                plan.clone()
            } else {
                let (scan_lower_bound, scan_upper_bound) =
                    self.source.table_bounds(table_name).await?;
                TableCopyPlan {
                    migration_id: self.source.migration_id().to_owned(),
                    table_name: table_name.clone(),
                    installation_id: self.installation_id.clone(),
                    source_identity: self.source.source_identity().to_owned(),
                    target_identity: self.target_identity.clone(),
                    scan_lower_bound,
                    scan_upper_bound,
                    capture_start,
                }
            };
            validate_table_bounds(
                "source migration table",
                &plan.scan_lower_bound,
                &plan.scan_upper_bound,
            )?;
            if existing.is_none() {
                self.target.save_plan(&plan).await?;
            }
            if let Some(checkpoint) = self
                .target
                .load_checkpoint(&plan.migration_id, &plan.table_name)
                .await?
            {
                self.validate_checkpoint_identity(&checkpoint)?;
                if checkpoint.migration_id != plan.migration_id
                    || checkpoint.table_name != plan.table_name
                    || checkpoint.capture_start != plan.capture_start
                    || checkpoint.scan_upper_bound != plan.scan_upper_bound
                {
                    bail!("durable migration checkpoint does not match its plan");
                }
                self.store
                    .save_checkpoint(&checkpoint.as_contract(self.authority_generation))
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn copy_next(&self, table_name: &str) -> Result<MigrationProgress> {
        self.ensure_running().await?;
        self.ensure_capture_ready().await?;
        let plan = self
            .target
            .load_plan(self.source.migration_id(), table_name)
            .await?
            .ok_or_else(|| anyhow!("migration plan is missing for table {table_name}"))?;
        self.validate_plan_identity(&plan)?;
        let checkpoint = self
            .target
            .load_checkpoint(&plan.migration_id, table_name)
            .await?
            .ok_or_else(|| anyhow!("migration checkpoint is missing for table {table_name}"))?;
        self.validate_checkpoint_identity(&checkpoint)?;
        if checkpoint.complete {
            return self.progress_for(&checkpoint).await;
        }
        let page = self
            .cancellable(tokio::time::timeout(
                self.limits.copy.source_read_budget,
                self.source.scan_page(
                    table_name,
                    checkpoint.cursor.as_deref(),
                    plan.scan_upper_bound.as_deref(),
                    self.limits.copy,
                ),
            ))
            .await
            .context("SQLite migration source page exceeded its read budget")??;
        page.validate(
            table_name,
            checkpoint.cursor.as_deref(),
            plan.scan_upper_bound.as_deref(),
            self.limits.copy,
        )?;
        let copied_rows = checkpoint
            .copied_rows
            .checked_add(page.rows.len() as u64)
            .ok_or_else(|| anyhow!("migration copied row count overflowed"))?;
        let copied_bytes = checkpoint
            .copied_bytes
            .checked_add(
                page.rows
                    .iter()
                    .map(|row| row.payload.len() as u64)
                    .sum::<u64>(),
            )
            .ok_or_else(|| anyhow!("migration copied byte count overflowed"))?;
        let next_checkpoint = EngineCheckpoint {
            cursor: page.next_cursor.clone(),
            copied_rows,
            copied_bytes,
            complete: page.complete,
            ..checkpoint
        };
        self.cancellable(
            self.target
                .apply_baseline_page(&plan, &page, &next_checkpoint),
        )
        .await?;
        self.cancellable(self.persist_checkpoint(&next_checkpoint))
            .await?;
        if !self.limits.copy.batch_yield.is_zero() {
            self.cancellable(async {
                tokio::time::sleep(self.limits.copy.batch_yield).await;
                Ok::<(), anyhow::Error>(())
            })
            .await?;
        }
        self.progress_for(&next_checkpoint).await
    }

    pub(crate) async fn catch_up(&self, table_name: &str) -> Result<MigrationProgress> {
        self.ensure_running().await?;
        let watermark = self.source.capture_watermark().await?;
        self.catch_up_through(table_name, watermark).await
    }

    pub(crate) async fn release_capture_through(&self, through: i64) -> Result<()> {
        self.ensure_running().await?;
        self.source.release_capture_through(through).await
    }

    async fn catch_up_through(
        &self,
        table_name: &str,
        watermark: i64,
    ) -> Result<MigrationProgress> {
        self.ensure_capture_ready().await?;
        if watermark < 0 {
            bail!("migration catch-up watermark must not be negative");
        }
        let Some(mut checkpoint) = self
            .target
            .load_checkpoint(self.source.migration_id(), table_name)
            .await?
        else {
            bail!("migration checkpoint is missing for table {table_name}");
        };
        self.validate_checkpoint_identity(&checkpoint)?;
        if checkpoint.applied_through > watermark {
            bail!("migration checkpoint is ahead of the requested catch-up watermark");
        }
        while checkpoint.applied_through < watermark {
            let changes = self
                .cancellable(self.source.changes_through(
                    checkpoint.applied_through,
                    watermark,
                    self.limits.change_batch,
                ))
                .await?;
            if changes.is_empty() {
                bail!("source capture skipped a required migration watermark prefix");
            }
            for change in &changes {
                change.validate_for(self.source.migration_id(), self.source.source_identity())?;
            }
            let through = changes
                .last()
                .map(|change| change.source_version)
                .ok_or_else(|| anyhow!("migration change batch is unexpectedly empty"))?;
            validate_contiguous_change_prefix(&changes, checkpoint.applied_through, watermark)?;
            if through <= checkpoint.applied_through || through > watermark {
                bail!("source capture returned an invalid migration watermark batch");
            }
            let table_changes = changes
                .iter()
                .filter(|change| change.table_name == table_name)
                .cloned()
                .collect::<Vec<_>>();
            checkpoint.captured_through = through;
            checkpoint.applied_through = through;
            self.cancellable(self.target.apply_change_batch(
                self.source.migration_id(),
                self.source.source_identity(),
                &table_changes,
                &checkpoint,
            ))
            .await?;
            self.cancellable(self.persist_checkpoint(&checkpoint))
                .await?;
        }
        // A target can commit the replay and lose the metadata acknowledgement; rewrite the
        // durable metadata checkpoint when a restart observes the committed target checkpoint.
        self.persist_checkpoint(&checkpoint).await?;
        self.progress_for(&checkpoint).await
    }

    pub(crate) async fn verify_chunk(
        &self,
        table_name: &str,
        range: ChunkRange,
    ) -> Result<ChunkVerification> {
        self.ensure_capture_ready().await?;
        let source_snapshot = self
            .cancellable(self.source.rows_in_range_snapshot(table_name, &range))
            .await?;
        let source_watermark = source_snapshot.source_watermark;
        let source_rows = source_snapshot.rows;
        let target_rows = self
            .cancellable(self.target.rows_in_range(table_name, &range))
            .await?;
        let source_digest = rows_digest(&source_rows);
        let target_digest = rows_digest(&target_rows);
        if source_rows != target_rows
            || source_digest != target_digest
            || self.cancellable(self.source.capture_watermark()).await? != source_watermark
        {
            return Ok(ChunkVerification::Dirty);
        }
        let proof = ChunkProof {
            migration_id: self.source.migration_id().to_owned(),
            table_name: table_name.to_owned(),
            installation_id: self.installation_id.clone(),
            source_identity: self.source.source_identity().to_owned(),
            target_identity: self.target_identity.clone(),
            range,
            source_watermark,
            row_count: source_rows.len() as u64,
            byte_count: source_rows.iter().map(|row| row.payload.len() as u64).sum(),
            digest: source_digest,
        };
        self.cancellable(self.target.save_chunk_proof(&proof))
            .await?;
        Ok(ChunkVerification::Verified)
    }

    pub(crate) async fn verify_table_coverage_at(
        &self,
        table_name: &str,
        source_watermark: i64,
    ) -> Result<TableCoverageProof> {
        self.ensure_capture_ready().await?;
        if source_watermark < 0 {
            bail!("migration coverage watermark must not be negative");
        }
        let source_bounds = self
            .cancellable(self.source.table_bounds(table_name))
            .await?;
        let target_bounds = self
            .cancellable(self.target.table_bounds(table_name))
            .await?;
        let (lower_bound, upper_bound) = union_table_bounds(
            &source_bounds.0,
            &source_bounds.1,
            &target_bounds.0,
            &target_bounds.1,
        )?;
        let range = lower_bound
            .clone()
            .zip(upper_bound.clone())
            .map(|(first, last)| ChunkRange::new(first, last))
            .transpose()?;
        let checkpoint = self
            .cancellable(
                self.target
                    .load_coverage_checkpoint(self.source.migration_id(), table_name),
            )
            .await?;
        if let Some(checkpoint) = &checkpoint {
            self.validate_coverage_identity(
                &checkpoint.migration_id,
                &checkpoint.installation_id,
                &checkpoint.source_identity,
                &checkpoint.target_identity,
            )?;
        }
        let resume = checkpoint.filter(|checkpoint| {
            !checkpoint.complete
                && checkpoint.lower_bound == lower_bound
                && checkpoint.upper_bound == upper_bound
                && checkpoint.source_watermark == source_watermark
        });
        let mut cursor = resume
            .as_ref()
            .and_then(|checkpoint| checkpoint.cursor.clone());
        let mut source_row_count = resume
            .as_ref()
            .map(|checkpoint| checkpoint.source_row_count)
            .unwrap_or_default();
        let mut target_row_count = resume
            .as_ref()
            .map(|checkpoint| checkpoint.target_row_count)
            .unwrap_or_default();
        let mut source_byte_count = resume
            .as_ref()
            .map(|checkpoint| checkpoint.source_byte_count)
            .unwrap_or_default();
        let mut target_byte_count = resume
            .as_ref()
            .map(|checkpoint| checkpoint.target_byte_count)
            .unwrap_or_default();
        let mut source_digest = resume
            .as_ref()
            .map(|checkpoint| checkpoint.source_digest.clone())
            .unwrap_or_else(|| rows_digest(&[]));
        let mut target_digest = resume
            .as_ref()
            .map(|checkpoint| checkpoint.target_digest.clone())
            .unwrap_or_else(|| rows_digest(&[]));
        let mut fixed_watermark = source_watermark;
        let mut watermark_restarts = 0_usize;
        loop {
            self.ensure_running().await?;
            let source_range_page = self
                .cancellable(tokio::time::timeout(
                    self.limits.copy.source_read_budget,
                    self.source.scan_range_page(
                        table_name,
                        cursor.as_deref(),
                        lower_bound.as_deref(),
                        upper_bound.as_deref(),
                        self.limits.copy,
                    ),
                ))
                .await
                .context("SQLite migration coverage page exceeded its read budget")??;
            source_range_page.page.validate_range(
                table_name,
                cursor.as_deref(),
                lower_bound.as_deref(),
                upper_bound.as_deref(),
                self.limits.copy,
            )?;
            if source_range_page.source_watermark != fixed_watermark {
                if source_range_page.source_watermark < fixed_watermark {
                    bail!("source coverage watermark regressed between bounded pages");
                }
                watermark_restarts = watermark_restarts
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("source coverage restart count overflowed"))?;
                if watermark_restarts > self.limits.max_queued_operations {
                    bail!("source changed too often during bounded coverage verification");
                }
                fixed_watermark = source_range_page.source_watermark;
                cursor = None;
                source_row_count = 0;
                target_row_count = 0;
                source_byte_count = 0;
                target_byte_count = 0;
                source_digest = rows_digest(&[]);
                target_digest = rows_digest(&[]);
                continue;
            }
            if let Some(checkpoint) = self
                .cancellable(
                    self.target
                        .load_checkpoint(self.source.migration_id(), table_name),
                )
                .await?
                && checkpoint.applied_through < fixed_watermark
            {
                self.catch_up_through(table_name, fixed_watermark).await?;
            }
            let target_page = self
                .cancellable(self.target.scan_range_page(
                    table_name,
                    cursor.as_deref(),
                    lower_bound.as_deref(),
                    upper_bound.as_deref(),
                    self.limits.copy,
                ))
                .await?;
            target_page.validate_range(
                table_name,
                cursor.as_deref(),
                lower_bound.as_deref(),
                upper_bound.as_deref(),
                self.limits.copy,
            )?;
            if source_range_page.page.rows != target_page.rows
                || source_range_page.page.next_cursor != target_page.next_cursor
                || source_range_page.page.complete != target_page.complete
            {
                bail!("migration table coverage differs between source and target");
            }
            source_row_count = source_row_count
                .checked_add(source_range_page.page.rows.len() as u64)
                .ok_or_else(|| anyhow!("source coverage row count overflowed"))?;
            target_row_count = target_row_count
                .checked_add(target_page.rows.len() as u64)
                .ok_or_else(|| anyhow!("target coverage row count overflowed"))?;
            let source_page_bytes = source_range_page
                .page
                .rows
                .iter()
                .map(|row| row.payload.len() as u64)
                .sum::<u64>();
            let target_page_bytes = target_page
                .rows
                .iter()
                .map(|row| row.payload.len() as u64)
                .sum::<u64>();
            source_byte_count = source_byte_count
                .checked_add(source_page_bytes)
                .ok_or_else(|| anyhow!("source coverage byte count overflowed"))?;
            target_byte_count = target_byte_count
                .checked_add(target_page_bytes)
                .ok_or_else(|| anyhow!("target coverage byte count overflowed"))?;
            source_digest = chain_digest(&source_digest, &source_range_page.page.rows);
            target_digest = chain_digest(&target_digest, &target_page.rows);
            let next_cursor = source_range_page.page.next_cursor.clone();
            self.cancellable(self.target.save_coverage_checkpoint(&CoverageCheckpoint {
                migration_id: self.source.migration_id().to_owned(),
                table_name: table_name.to_owned(),
                installation_id: self.installation_id.clone(),
                source_identity: self.source.source_identity().to_owned(),
                target_identity: self.target_identity.clone(),
                lower_bound: lower_bound.clone(),
                upper_bound: upper_bound.clone(),
                cursor: next_cursor.clone(),
                source_watermark: fixed_watermark,
                source_row_count,
                target_row_count,
                source_byte_count,
                target_byte_count,
                source_digest: source_digest.clone(),
                target_digest: target_digest.clone(),
                complete: source_range_page.page.complete,
            }))
            .await?;
            if source_range_page.page.complete {
                break;
            }
            if next_cursor.as_deref() == cursor.as_deref() {
                bail!("migration coverage page did not advance");
            }
            cursor = next_cursor;
        }
        let proof = TableCoverageProof {
            migration_id: self.source.migration_id().to_owned(),
            table_name: table_name.to_owned(),
            installation_id: self.installation_id.clone(),
            source_identity: self.source.source_identity().to_owned(),
            target_identity: self.target_identity.clone(),
            source_watermark: fixed_watermark,
            range,
            source_lower_bound: source_bounds.0,
            source_upper_bound: source_bounds.1,
            target_lower_bound: target_bounds.0,
            target_upper_bound: target_bounds.1,
            source_row_count,
            target_row_count,
            source_byte_count,
            target_byte_count,
            source_digest,
            target_digest,
        };
        proof.validate()?;
        self.cancellable(self.target.save_table_coverage_proof(&proof))
            .await?;
        Ok(proof)
    }

    async fn verify_tail_delta(
        &self,
        tables: &BTreeSet<String>,
        after: i64,
        through: i64,
    ) -> Result<()> {
        self.ensure_capture_ready().await?;
        if after < 0 || through < after {
            bail!("migration tail delta watermark is invalid");
        }
        if after == through {
            return Ok(());
        }
        let changes = self
            .cancellable(
                self.source
                    .changes_through(after, through, self.limits.change_batch),
            )
            .await?;
        for change in &changes {
            change.validate_for(self.source.migration_id(), self.source.source_identity())?;
        }
        validate_contiguous_change_prefix(&changes, after, through)?;
        let last_change = changes
            .iter()
            .map(|change| change.source_version)
            .max()
            .ok_or_else(|| anyhow!("source capture skipped the migration tail delta"))?;
        if last_change != through {
            bail!("migration tail delta exceeds the bounded replay budget");
        }
        for table_name in tables {
            self.ensure_running().await?;
            if let Some(checkpoint) = self
                .cancellable(
                    self.target
                        .load_checkpoint(self.source.migration_id(), table_name),
                )
                .await?
                && checkpoint.applied_through < through
            {
                self.catch_up_through(table_name, through).await?;
            }
            let mut keys = BTreeSet::new();
            for change in changes
                .iter()
                .filter(|change| &change.table_name == table_name)
            {
                keys.insert(change.stable_key.clone());
                if let Some(previous_key) = &change.previous_key {
                    keys.insert(previous_key.clone());
                }
            }
            for stable_key in keys {
                self.verify_key_delta(table_name, &stable_key, through)
                    .await?;
            }
        }
        Ok(())
    }

    async fn verify_key_delta(
        &self,
        table_name: &str,
        stable_key: &str,
        through: i64,
    ) -> Result<()> {
        let source_range_page = self
            .cancellable(tokio::time::timeout(
                self.limits.copy.source_read_budget,
                self.source.scan_range_page(
                    table_name,
                    None,
                    Some(stable_key),
                    Some(stable_key),
                    self.limits.copy,
                ),
            ))
            .await
            .context("SQLite migration tail verification exceeded its read budget")??;
        source_range_page.page.validate_range(
            table_name,
            None,
            Some(stable_key),
            Some(stable_key),
            self.limits.copy,
        )?;
        if source_range_page.source_watermark < through {
            bail!("source tail verification page precedes its watermark");
        }
        if let Some(checkpoint) = self
            .target
            .load_checkpoint(self.source.migration_id(), table_name)
            .await?
            && checkpoint.applied_through < source_range_page.source_watermark
        {
            self.catch_up_through(table_name, source_range_page.source_watermark)
                .await?;
        }
        let target_page = self
            .cancellable(self.target.scan_range_page(
                table_name,
                None,
                Some(stable_key),
                Some(stable_key),
                self.limits.copy,
            ))
            .await?;
        target_page.validate_range(
            table_name,
            None,
            Some(stable_key),
            Some(stable_key),
            self.limits.copy,
        )?;
        if source_range_page.page.rows != target_page.rows
            || source_range_page.page.next_cursor != target_page.next_cursor
            || source_range_page.page.complete != target_page.complete
        {
            bail!("migration tail delta differs between source and target");
        }
        Ok(())
    }

    pub(crate) async fn switch_authority(
        &self,
        gate: &ModuleGenerationGate,
        expected: AuthorityRecoveryContext,
        switch_budget: Duration,
    ) -> Result<()> {
        let migration = (*self).clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            migration
                .switch_authority_inner(&gate, expected, switch_budget)
                .await
        })
        .await
        .context("migration authority switch worker failed")?
    }

    async fn switch_authority_inner(
        &self,
        gate: &ModuleGenerationGate,
        expected: AuthorityRecoveryContext,
        switch_budget: Duration,
    ) -> Result<()> {
        let _switch = self.switching.lock().await;
        self.ensure_running().await?;
        self.ensure_capture_ready().await?;
        if expected.migration_id != self.source.migration_id()
            || expected.source_identity != self.source.source_identity()
            || expected.installation_id != self.installation_id
            || expected.target_identity != self.target_identity
            || expected.module != self.module
        {
            bail!("migration authority context does not match the engine");
        }
        let pre_fence_watermark = self.source.capture_watermark().await?;
        if expected.source_watermark > pre_fence_watermark {
            bail!("migration authority context is ahead of the current source watermark");
        }
        let current_authority = gate.current_authority().await;
        if current_authority.backend != BackendKind::Sqlite
            || current_authority.generation != self.authority_generation
        {
            bail!("migration authority switch does not match the old SQLite generation");
        }
        let tables = self.target.known_tables(self.source.migration_id()).await?;
        let source_tables = self
            .source
            .module_tables(self.module)
            .await?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let target_tables = tables.into_iter().collect::<BTreeSet<_>>();
        if source_tables.is_empty() || target_tables != source_tables {
            bail!("migration authority switch requires an exact prepared table set");
        }
        for table_name in &target_tables {
            if self
                .target
                .load_plan(self.source.migration_id(), table_name)
                .await?
                .is_none()
            {
                bail!("migration authority switch requires a prepared plan for {table_name}");
            }
            let progress = self
                .catch_up_through(table_name, pre_fence_watermark)
                .await?;
            if !progress.complete {
                bail!("migration table {table_name} is not complete before switch");
            }
            self.verify_table_coverage_at(table_name, pre_fence_watermark)
                .await?;
        }

        self.fence.close_and_drain(switch_budget).await?;
        let next_authority_generation = self
            .authority_generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("migration authority generation exhausted"))?;
        let mut source_fenced = false;
        let result = async {
            // The in-memory fence is shared with routed production writers. Persist the same
            // generation-bound fence before reading the post-fence watermark so direct SQLite
            // writers are rejected by the trigger before the final capture boundary is chosen.
            self.source
                .fence_source_writes(self.authority_generation, next_authority_generation)
                .await?;
            source_fenced = true;
            let watermark = self.source.capture_watermark().await?;
            if watermark < pre_fence_watermark {
                bail!("source capture watermark regressed during the switch fence");
            }
            if watermark > pre_fence_watermark {
                tokio::time::timeout(
                    switch_budget,
                    self.verify_tail_delta(&target_tables, pre_fence_watermark, watermark),
                )
                .await
                .context("migration switch tail delta exceeded its budget")??;
            }
            let final_watermark = self.source.capture_watermark().await?;
            if final_watermark != watermark {
                bail!("source capture advanced during the switch tail verification");
            }
            let final_expected = AuthorityRecoveryContext {
                source_watermark: final_watermark,
                ..expected.clone()
            };
            self.store.set_recovery_context(&final_expected).await?;
            let manifest = ModuleAuthorityManifest {
                migration_id: final_expected.migration_id.clone(),
                installation_id: final_expected.installation_id.clone(),
                source_identity: final_expected.source_identity.clone(),
                module: final_expected.module,
                authority: ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: next_authority_generation,
                },
                source_watermark: final_watermark,
                target_identity: final_expected.target_identity.clone(),
            };
            let metadata: Arc<dyn AuthorityManifestStore> = self.store.clone();
            gate.switch_authority(
                self.module,
                manifest,
                final_expected,
                switch_budget,
                metadata,
            )
            .await
        }
        .await;
        if result.is_err() && !gate.requires_recovery() {
            let authority = gate.current_authority().await;
            if authority.backend == BackendKind::Sqlite
                && authority.generation == self.authority_generation
            {
                if source_fenced {
                    self.source
                        .reopen_source_writes(next_authority_generation)
                        .await?;
                }
                self.fence.reopen().await;
            }
        }
        result
    }

    async fn ensure_running(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            bail!("storage migration engine cancelled");
        }
        if self.run_control.is_paused() {
            bail!("migration engine is paused");
        }
        Ok(())
    }

    async fn persist_checkpoint(&self, checkpoint: &EngineCheckpoint) -> Result<()> {
        self.store
            .save_checkpoint(&checkpoint.as_contract(self.authority_generation))
            .await
    }

    async fn progress_for(&self, checkpoint: &EngineCheckpoint) -> Result<MigrationProgress> {
        let watermark = self.source.capture_watermark().await?;
        Ok(MigrationProgress {
            table_name: checkpoint.table_name.clone(),
            copied_rows: checkpoint.copied_rows,
            copied_bytes: checkpoint.copied_bytes,
            cursor: checkpoint.cursor.clone(),
            applied_through: checkpoint.applied_through,
            pending_changes: watermark
                .saturating_sub(checkpoint.applied_through)
                .try_into()
                .unwrap_or(usize::MAX),
            complete: checkpoint.complete,
        })
    }
}

fn rows_digest(rows: &[VersionedRow]) -> String {
    let mut digest = Sha256::new();
    for row in rows {
        digest.update((row.stable_key.len() as u64).to_be_bytes());
        digest.update(row.stable_key.as_bytes());
        digest.update(row.source_version.to_be_bytes());
        digest.update((row.payload.len() as u64).to_be_bytes());
        digest.update(&row.payload);
    }
    encode_hex(&digest.finalize())
}

fn chain_digest(previous: &str, rows: &[VersionedRow]) -> String {
    let mut digest = Sha256::new();
    digest.update((previous.len() as u64).to_be_bytes());
    digest.update(previous.as_bytes());
    digest.update((rows.len() as u64).to_be_bytes());
    digest.update(
        rows.iter()
            .map(|row| row.payload.len() as u64)
            .sum::<u64>()
            .to_be_bytes(),
    );
    digest.update(rows_digest(rows).as_bytes());
    encode_hex(&digest.finalize())
}
