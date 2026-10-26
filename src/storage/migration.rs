use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    future::Future,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

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
    fn from_input(
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
        Ok(Self {
            migration_id: migration_id.to_owned(),
            source_identity: source_identity.to_owned(),
            table_name: input.table_name,
            stable_key: input.stable_key,
            previous_key: input.previous_key,
            kind: input.kind,
            source_version,
            payload_digest: payload_digest(input.payload.as_deref()),
            payload: input.payload,
        })
    }

    fn validate_for(&self, migration_id: &str, source_identity: &str) -> Result<()> {
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

#[derive(Debug, Clone)]
pub(crate) struct VersionCapture {
    state: Arc<Mutex<VersionCaptureState>>,
    migration_id: String,
    source_identity: String,
}

#[derive(Debug, Default)]
struct VersionCaptureState {
    next_version: i64,
    changes: Vec<CapturedChange>,
    continuity_lost: bool,
}

impl VersionCapture {
    pub(crate) fn new(migration_id: impl Into<String>, source_identity: impl Into<String>) -> Self {
        Self {
            state: Arc::new(Mutex::new(VersionCaptureState {
                next_version: 0,
                changes: Vec::new(),
                continuity_lost: false,
            })),
            migration_id: migration_id.into(),
            source_identity: source_identity.into(),
        }
    }

    async fn append(&self, changes: Vec<ChangeInput>) -> Result<i64> {
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
        state.next_version = version;
        state.changes.extend(captured);
        Ok(version)
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
        if through < after {
            return Ok(Vec::new());
        }
        self.changes_between(after, Some(through), limit).await
    }

    async fn changes_between(
        &self,
        after: i64,
        through: Option<i64>,
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        if after < 0 || limit == 0 {
            bail!("source change cursor and limit must be positive");
        }
        let state = self.state.lock().await;
        if state.continuity_lost {
            bail!("source change capture continuity has been lost");
        }
        let mut selected = Vec::new();
        let mut selected_version = None;
        for change in state.changes.iter().filter(|change| {
            change.source_version > after
                && through.is_none_or(|watermark| change.source_version <= watermark)
        }) {
            if selected_version != Some(change.source_version) {
                if !selected.is_empty() && selected.len() + 1 > limit {
                    break;
                }
                selected_version = Some(change.source_version);
            }
            selected.push(change.clone());
        }
        Ok(selected)
    }

    pub(crate) async fn mark_continuity_lost(&self) {
        self.state.lock().await.continuity_lost = true;
    }

    pub(crate) async fn capture_transaction(&self, changes: Vec<ChangeInput>) -> Result<i64> {
        self.append(changes).await
    }

    pub(crate) fn migration_id(&self) -> &str {
        &self.migration_id
    }

    pub(crate) fn source_identity(&self) -> &str {
        &self.source_identity
    }
}

fn validate_change_batch(changes: &[CapturedChange], after: i64, through: i64) -> Result<()> {
    if after < 0 || through < after {
        bail!("source change batch watermark range is invalid");
    }
    if changes.is_empty() {
        if through > after {
            bail!("source change batch skipped a required version prefix");
        }
        return Ok(());
    }
    let mut previous = after;
    for (index, change) in changes.iter().enumerate() {
        if (index == 0 && change.source_version != after.saturating_add(1))
            || change.source_version < previous
            || change.source_version > through
            || change.source_version > previous.saturating_add(1)
        {
            bail!("source change batch is outside its requested watermark range");
        }
        previous = change.source_version;
    }
    Ok(())
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

impl SourcePage {
    fn validate(
        &self,
        table_name: &str,
        cursor: Option<&str>,
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
            if !self.complete {
                bail!("an empty source page must be complete");
            }
            if self.next_cursor.is_some() {
                bail!("an empty source page cannot advance its cursor");
            }
            return Ok(());
        }

        let mut previous = cursor;
        for row in &self.rows {
            if row.stable_key.trim().is_empty() || row.source_version < 0 {
                bail!("source page contains an invalid row");
            }
            if previous.is_some_and(|previous| row.stable_key.as_str() <= previous) {
                bail!("source page rows are not strictly key ordered");
            }
            if upper_bound.is_some_and(|upper| row.stable_key.as_str() > upper) {
                bail!("source page row exceeds its scan upper bound");
            }
            previous = Some(row.stable_key.as_str());
        }
        if self.next_cursor.as_deref() != self.rows.last().map(|row| row.stable_key.as_str()) {
            bail!("source page cursor does not match its final row");
        }
        if self.complete
            && upper_bound.is_some_and(|upper| self.next_cursor.as_deref() != Some(upper))
        {
            bail!("a complete source page must reach its scan upper bound");
        }
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
    pub(crate) scan_lower_bound: Option<String>,
    pub(crate) scan_upper_bound: Option<String>,
    pub(crate) capture_start: i64,
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
    fn validate(self) -> Result<()> {
        if self.max_rows == 0 || self.max_bytes == 0 {
            bail!("migration copy limits must be positive");
        }
        if self.source_read_budget.is_zero() {
            bail!("migration source read budget must be positive");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EngineCheckpoint {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
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
    fn as_contract(&self, source_identity: &str, generation: u64) -> MigrationCheckpoint {
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
            source_identity: source_identity.to_owned(),
            authority_generation: generation,
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
    pub(crate) range: ChunkRange,
    pub(crate) source_watermark: i64,
    pub(crate) row_count: u64,
    pub(crate) byte_count: u64,
    pub(crate) digest: String,
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
    async fn table_lower_bound(&self, table_name: &str) -> Result<Option<String>>;
    async fn table_upper_bound(&self, table_name: &str) -> Result<Option<String>>;
    async fn scan_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage>;
    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>>;
    async fn changes_after(&self, after: i64, limit: usize) -> Result<Vec<CapturedChange>>;
    async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<CapturedChange>>;
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
        changes: &[CapturedChange],
        checkpoint: &EngineCheckpoint,
    ) -> Result<()>;
    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>>;
    async fn save_chunk_proof(&self, proof: &ChunkProof) -> Result<()>;
    async fn verified_chunks(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Vec<ChunkProof>>;
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
    pub(crate) max_queued_operations: usize,
    pub(crate) min_source_free_bytes: u64,
    pub(crate) max_capture_bytes: u64,
}

impl Default for MigrationLimits {
    fn default() -> Self {
        Self {
            copy: CopyLimits::default(),
            max_queued_operations: DEFAULT_MAX_QUEUED_OPERATIONS,
            min_source_free_bytes: DEFAULT_MIN_SOURCE_FREE_BYTES,
            max_capture_bytes: DEFAULT_MAX_CAPTURE_BYTES,
        }
    }
}

impl MigrationLimits {
    fn validate(self) -> Result<()> {
        self.copy.validate()?;
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
    state: Arc<Mutex<RunControlState>>,
    changed: Arc<Notify>,
}

#[derive(Debug, Default)]
struct RunControlState {
    paused: bool,
    attempt: u32,
    last_error: Option<String>,
}

impl Default for MigrationRunControl {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(RunControlState::default())),
            changed: Arc::new(Notify::new()),
        }
    }
}

impl MigrationRunControl {
    pub(crate) async fn pause(&self, reason: impl Into<String>) {
        let mut state = self.state.lock().await;
        state.paused = true;
        state.last_error = Some(reason.into());
        self.changed.notify_waiters();
    }

    pub(crate) async fn resume(&self) {
        let mut state = self.state.lock().await;
        state.paused = false;
        state.last_error = None;
        self.changed.notify_waiters();
    }

    pub(crate) async fn is_paused(&self) -> bool {
        self.state.lock().await.paused
    }

    pub(crate) async fn check(&self) -> Result<()> {
        let state = self.state.lock().await;
        if state.paused {
            bail!(
                "migration is paused: {}",
                state.last_error.as_deref().unwrap_or("operator hold")
            );
        }
        Ok(())
    }

    pub(crate) async fn record_failure(&self, error: &anyhow::Error) -> Duration {
        let mut state = self.state.lock().await;
        state.attempt = state.attempt.saturating_add(1);
        state.last_error = Some(error.to_string());
        retry_delay(state.attempt)
    }

    pub(crate) async fn record_success(&self) {
        let mut state = self.state.lock().await;
        state.attempt = 0;
        state.last_error = None;
    }

    pub(crate) async fn retry_attempts(&self) -> u32 {
        self.state.lock().await.attempt
    }

    pub(crate) async fn wait_until_resumed(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !self.is_paused().await {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) fn retry_delay(attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(5);
    let multiplier = 1_u32 << shift;
    DEFAULT_RETRY_INITIAL
        .checked_mul(multiplier)
        .unwrap_or(DEFAULT_RETRY_MAX)
        .min(DEFAULT_RETRY_MAX)
}

#[derive(Debug, Clone)]
pub(crate) struct SourceWriteFence {
    state: Arc<Mutex<SourceFenceState>>,
    changed: Arc<Notify>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceFenceState {
    accepting: bool,
    fenced: bool,
    active_writes: usize,
    generation: u64,
}

#[derive(Debug)]
pub(crate) struct SourceWriteLease {
    fence: SourceWriteFence,
}

#[derive(Debug)]
pub(crate) struct SourceFenceLease {
    fence: SourceWriteFence,
    previous_generation: u64,
    prepared_generation: Option<u64>,
    completed: bool,
}

impl SourceWriteFence {
    pub(crate) fn new(generation: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(SourceFenceState {
                accepting: true,
                fenced: false,
                active_writes: 0,
                generation,
            })),
            changed: Arc::new(Notify::new()),
        }
    }

    pub(crate) async fn begin_write(&self) -> Result<SourceWriteLease> {
        let mut state = self.state.lock().await;
        if !state.accepting || state.fenced {
            bail!("source writes are fenced for migration cutover");
        }
        state.active_writes = state.active_writes.saturating_add(1);
        Ok(SourceWriteLease {
            fence: self.clone(),
        })
    }

    pub(crate) async fn close_and_drain(&self, budget: Duration) -> Result<SourceFenceLease> {
        let deadline = tokio::time::Instant::now() + budget;
        let previous_generation = {
            let mut state = self.state.lock().await;
            if state.fenced {
                bail!("source writes are already fenced");
            }
            state.accepting = false;
            state.generation
        };
        let lease = SourceFenceLease {
            fence: self.clone(),
            previous_generation,
            prepared_generation: None,
            completed: false,
        };
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.state.lock().await.active_writes == 0 {
                break;
            }
            if tokio::time::timeout_at(deadline, notified.as_mut())
                .await
                .is_err()
            {
                lease.reopen().await;
                bail!("timed out draining source writes before authority publication");
            }
        }
        Ok(lease)
    }

    pub(crate) async fn reopen(&self) {
        let mut state = self.state.lock().await;
        if !state.fenced {
            state.accepting = true;
        }
        self.changed.notify_waiters();
    }

    async fn reopen_uncommitted(&self, previous_generation: u64, prepared_generation: u64) {
        let mut state = self.state.lock().await;
        if state.fenced && state.generation == prepared_generation {
            state.generation = previous_generation;
            state.fenced = false;
            state.accepting = true;
        }
        self.changed.notify_waiters();
    }

    pub(crate) async fn commit(&self, generation: u64) -> Result<()> {
        let mut state = self.state.lock().await;
        if generation <= state.generation {
            bail!("source fence generation must advance");
        }
        if state.accepting || state.active_writes != 0 {
            bail!("source fence requires closed, drained writes");
        }
        state.fenced = true;
        state.generation = generation;
        self.changed.notify_waiters();
        Ok(())
    }

    pub(crate) async fn recover(&self, authority: BackendKind, generation: u64) -> Result<()> {
        let mut state = self.state.lock().await;
        if state.active_writes != 0 {
            bail!("cannot recover source fence while writes are active");
        }
        if generation < state.generation && authority != BackendKind::Sqlite {
            bail!("source fence recovery regresses generation");
        }
        state.generation = generation;
        state.fenced = authority == BackendKind::Postgres;
        state.accepting = !state.fenced;
        self.changed.notify_waiters();
        Ok(())
    }

    pub(crate) async fn is_fenced(&self) -> bool {
        self.state.lock().await.fenced
    }

    pub(crate) async fn active_writes(&self) -> usize {
        self.state.lock().await.active_writes
    }
}

impl SourceWriteLease {
    pub(crate) fn complete(self) {
        // Consuming the lease releases it through Drop after the source transaction commits.
    }
}

impl Drop for SourceWriteLease {
    fn drop(&mut self) {
        self.release();
    }
}

impl SourceWriteLease {
    fn release(&mut self) {
        if let Ok(mut state) = self.fence.state.try_lock() {
            state.active_writes = state.active_writes.saturating_sub(1);
            self.fence.changed.notify_waiters();
        } else {
            let fence = self.fence.clone();
            tokio::spawn(async move {
                let mut state = fence.state.lock().await;
                state.active_writes = state.active_writes.saturating_sub(1);
                fence.changed.notify_waiters();
            });
        }
    }
}

impl SourceFenceLease {
    pub(crate) async fn commit(&mut self, generation: u64) -> Result<()> {
        if self.prepared_generation.is_some() {
            bail!("source fence has already been prepared for authority publication");
        }
        self.fence.commit(generation).await?;
        self.prepared_generation = Some(generation);
        Ok(())
    }

    pub(crate) async fn reopen(mut self) {
        if let Some(generation) = self.prepared_generation {
            self.fence
                .reopen_uncommitted(self.previous_generation, generation)
                .await;
        } else {
            self.fence.reopen().await;
        }
        self.completed = true;
    }

    pub(crate) fn finalize(mut self) {
        self.completed = true;
    }
}

impl Drop for SourceFenceLease {
    fn drop(&mut self) {
        if !self.completed {
            if self.prepared_generation.is_some() {
                return;
            }
            let fence = self.fence.clone();
            tokio::spawn(async move { fence.reopen().await });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SourceFenceRecord {
    pub(crate) migration_id: String,
    pub(crate) module: StorageModule,
    pub(crate) source_identity: String,
    pub(crate) generation: u64,
    pub(crate) fenced: bool,
}

#[async_trait]
pub(crate) trait MigrationFenceStore: Send + Sync {
    async fn load_source_fence(&self, migration_id: &str) -> Result<Option<SourceFenceRecord>>;
    async fn save_source_fence(&self, fence: &SourceFenceRecord) -> Result<()>;
    async fn prepare_authority_context(&self, expected: &AuthorityRecoveryContext) -> Result<()>;
}

#[derive(Debug, Clone)]
pub(crate) struct MigrationEngine<S, T, M>
where
    S: MigrationSource,
    T: MigrationTarget,
    M: MigrationMetadataStore + MigrationFenceStore,
{
    source: Arc<S>,
    target: Arc<T>,
    metadata: Arc<M>,
    limits: MigrationLimits,
    control: MigrationRunControl,
    fence: SourceWriteFence,
    module: StorageModule,
    target_identity: String,
    installation_id: String,
    authority_generation: u64,
}

impl<S, T, M> MigrationEngine<S, T, M>
where
    S: MigrationSource + 'static,
    T: MigrationTarget + 'static,
    M: MigrationMetadataStore + MigrationFenceStore + 'static,
{
    pub(crate) fn new(
        source: Arc<S>,
        target: Arc<T>,
        metadata: Arc<M>,
        module: StorageModule,
        installation_id: impl Into<String>,
        target_identity: impl Into<String>,
        authority_generation: u64,
        limits: MigrationLimits,
        fence: SourceWriteFence,
    ) -> Result<Self> {
        limits.validate()?;
        let installation_id = installation_id.into();
        if installation_id.trim().is_empty() {
            bail!("migration engine requires an installation identity");
        }
        let target_identity = target_identity.into();
        if target_identity.trim().is_empty() || authority_generation == 0 {
            bail!("migration engine requires target identity and generation");
        }
        if source.migration_id().trim().is_empty() || source.source_identity().trim().is_empty() {
            bail!("migration source requires migration and source identity");
        }
        Ok(Self {
            source,
            target,
            metadata,
            limits,
            control: MigrationRunControl::default(),
            fence,
            module,
            target_identity,
            installation_id,
            authority_generation,
        })
    }

    pub(crate) fn control(&self) -> MigrationRunControl {
        self.control.clone()
    }

    pub(crate) fn fence(&self) -> SourceWriteFence {
        self.fence.clone()
    }

    pub(crate) async fn pause(&self, reason: impl Into<String>) -> Result<()> {
        let reason = reason.into();
        self.control.pause(reason.clone()).await;
        let Some(mut status) = self.metadata.load_status(self.module).await? else {
            return Ok(());
        };
        if status.state.is_terminal() {
            return Ok(());
        }
        status.hold_reason = Some(reason.clone());
        let now = epoch_ms().max(status.updated_at_epoch_ms);
        status.transition_to(MigrationState::Paused, now)?;
        self.metadata.save_status(&status).await
    }

    pub(crate) async fn resume_to(&self, next: MigrationState) -> Result<()> {
        self.control.resume().await;
        let Some(mut status) = self.metadata.load_status(self.module).await? else {
            return Ok(());
        };
        if status.state != MigrationState::Paused {
            return Ok(());
        }
        status.hold_reason = None;
        let now = epoch_ms().max(status.updated_at_epoch_ms);
        status.transition_to(next, now)?;
        self.metadata.save_status(&status).await
    }

    pub(crate) async fn retry<Value, F, Fut>(
        &self,
        max_attempts: u32,
        mut operation: F,
    ) -> Result<Value>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<Value>>,
    {
        if max_attempts == 0 {
            bail!("migration retry attempts must be positive");
        }
        for attempt in 0..max_attempts {
            self.control.wait_until_resumed().await;
            match operation().await {
                Ok(value) => {
                    self.control.record_success().await;
                    return Ok(value);
                }
                Err(error) if attempt + 1 == max_attempts => {
                    if let Err(pause_error) = self.pause(error.to_string()).await {
                        return Err(error.context(format!(
                            "migration retry exhausted and pause persistence failed: {pause_error}"
                        )));
                    }
                    return Err(error);
                }
                Err(error) => {
                    let delay = self.control.record_failure(&error).await;
                    tokio::time::sleep(delay).await;
                }
            }
        }
        unreachable!("migration retry loop always returns")
    }

    pub(crate) async fn prepare(&self, tables: &[String]) -> Result<()> {
        self.control.check().await?;
        if tables.is_empty() {
            bail!("migration requires at least one table");
        }
        if tables.iter().any(|table| table.trim().is_empty()) {
            bail!("migration table names must not be empty");
        }
        let status = self.metadata.load_status(self.module).await?;
        if status
            .as_ref()
            .is_some_and(|status| status.state == MigrationState::Postgres)
        {
            bail!("migration module already uses PostgreSQL authority");
        }
        if status.as_ref().is_none_or(|status| {
            matches!(
                status.state,
                MigrationState::Pending | MigrationState::Paused | MigrationState::Blocked
            )
        }) {
            self.save_status(MigrationState::Preparing, None).await?;
        }
        let capture_start = self.source.capture_watermark().await?;
        for table_name in tables {
            if self
                .target
                .load_plan(self.source.migration_id(), table_name)
                .await?
                .is_some()
            {
                continue;
            }
            let plan = TableCopyPlan {
                migration_id: self.source.migration_id().to_owned(),
                table_name: table_name.clone(),
                scan_lower_bound: self.source.table_lower_bound(table_name).await?,
                scan_upper_bound: self.source.table_upper_bound(table_name).await?,
                capture_start,
            };
            self.target.save_plan(&plan).await?;
            self.target
                .apply_baseline_page(
                    &plan,
                    &SourcePage {
                        rows: Vec::new(),
                        next_cursor: None,
                        complete: false,
                    },
                    &EngineCheckpoint {
                        migration_id: self.source.migration_id().to_owned(),
                        table_name: table_name.clone(),
                        cursor: None,
                        scan_upper_bound: plan.scan_upper_bound.clone(),
                        capture_start,
                        captured_through: capture_start,
                        applied_through: capture_start,
                        copied_rows: 0,
                        copied_bytes: 0,
                        complete: false,
                    },
                )
                .await?;
        }
        if self
            .metadata
            .load_status(self.module)
            .await?
            .is_some_and(|status| status.state == MigrationState::Preparing)
        {
            self.save_status(MigrationState::Copying, None).await?;
        }
        Ok(())
    }

    pub(crate) async fn copy_next(&self, table_name: &str) -> Result<MigrationProgress> {
        self.control.check().await?;
        let plan = self
            .target
            .load_plan(self.source.migration_id(), table_name)
            .await?
            .ok_or_else(|| anyhow!("migration table has not been prepared: {table_name}"))?;
        let current = self
            .target
            .load_checkpoint(self.source.migration_id(), table_name)
            .await?
            .ok_or_else(|| anyhow!("migration checkpoint is missing for {table_name}"))?;
        if current.complete {
            return self.progress_for(&current).await;
        }
        let started = tokio::time::Instant::now();
        let page = self
            .source
            .scan_page(
                table_name,
                current.cursor.as_deref(),
                plan.scan_upper_bound.as_deref(),
                self.limits.copy,
            )
            .await?;
        if started.elapsed() > self.limits.copy.source_read_budget {
            self.pause("source_read_budget").await?;
            bail!("migration source read exceeded its budget");
        }
        page.validate(
            table_name,
            current.cursor.as_deref(),
            plan.scan_upper_bound.as_deref(),
            self.limits.copy,
        )?;
        let next = EngineCheckpoint {
            migration_id: current.migration_id.clone(),
            table_name: current.table_name.clone(),
            cursor: page.next_cursor.clone(),
            scan_upper_bound: current.scan_upper_bound.clone(),
            capture_start: current.capture_start,
            captured_through: current.captured_through,
            applied_through: current.applied_through,
            copied_rows: current.copied_rows.saturating_add(page.rows.len() as u64),
            copied_bytes: current.copied_bytes.saturating_add(
                page.rows
                    .iter()
                    .map(|row| row.payload.len() as u64)
                    .sum::<u64>(),
            ),
            complete: page.complete,
        };
        self.target.apply_baseline_page(&plan, &page, &next).await?;
        self.persist_checkpoint(&next).await?;
        tokio::time::sleep(self.limits.copy.batch_yield).await;
        self.progress_for(&next).await
    }

    pub(crate) async fn catch_up(&self, table_name: &str) -> Result<MigrationProgress> {
        let through = self.source.capture_watermark().await?;
        self.catch_up_through(table_name, through).await
    }

    async fn catch_up_through(&self, table_name: &str, through: i64) -> Result<MigrationProgress> {
        self.control.check().await?;
        let current = self
            .target
            .load_checkpoint(self.source.migration_id(), table_name)
            .await?
            .ok_or_else(|| anyhow!("migration checkpoint is missing for {table_name}"))?;
        if !current.complete {
            bail!("baseline copy is incomplete for {table_name}");
        }
        if through < 0 {
            bail!("source watermark must not be negative");
        }
        if through < current.applied_through {
            bail!("source watermark regressed below the durable migration checkpoint");
        }
        let changes = self
            .source
            .changes_through(current.applied_through, through, self.limits.copy.max_rows)
            .await?;
        let prepared_tables = self.prepared_tables().await?;
        for change in &changes {
            change.validate_for(self.source.migration_id(), self.source.source_identity())?;
            if !prepared_tables
                .iter()
                .any(|table| table == &change.table_name)
            {
                bail!("source change belongs to an unprepared migration table");
            }
        }
        validate_change_batch(&changes, current.applied_through, through)?;
        if changes.is_empty() && current.applied_through >= through {
            return self.progress_for(&current).await;
        }
        let applied_through = changes
            .last()
            .map(|change| change.source_version)
            .unwrap_or(through);
        let table_changes = changes
            .iter()
            .filter(|change| change.table_name == table_name)
            .cloned()
            .collect::<Vec<_>>();
        let next = EngineCheckpoint {
            captured_through: applied_through,
            applied_through,
            ..current.clone()
        };
        self.target
            .apply_change_batch(self.source.migration_id(), &table_changes, &next)
            .await?;
        self.persist_checkpoint(&next).await?;
        self.progress_for(&next).await
    }

    pub(crate) async fn verify_chunk(
        &self,
        table_name: &str,
        range: ChunkRange,
    ) -> Result<ChunkVerification> {
        self.control.check().await?;
        let plan = self
            .target
            .load_plan(self.source.migration_id(), table_name)
            .await?
            .ok_or_else(|| anyhow!("migration table has not been prepared: {table_name}"))?;
        let before = self.source.capture_watermark().await?;
        let source_rows = self.source.rows_in_range(table_name, &range).await?;
        let target_rows = self.target.rows_in_range(table_name, &range).await?;
        let after = self.source.capture_watermark().await?;
        if before != after {
            return Ok(ChunkVerification::Dirty);
        }
        let source_digest = normalized_rows_digest(&source_rows);
        let target_digest = normalized_rows_digest(&target_rows);
        if source_digest != target_digest || source_rows.len() != target_rows.len() {
            return Ok(ChunkVerification::Dirty);
        }
        let proof = ChunkProof {
            migration_id: self.source.migration_id().to_owned(),
            table_name: table_name.to_owned(),
            range,
            source_watermark: after,
            row_count: source_rows.len() as u64,
            byte_count: source_rows.iter().map(|row| row.payload.len() as u64).sum(),
            digest: source_digest,
        };
        if plan.migration_id != proof.migration_id || plan.table_name != proof.table_name {
            bail!("migration chunk proof does not match its table copy plan");
        }
        self.target.save_chunk_proof(&proof).await?;
        Ok(ChunkVerification::Verified)
    }

    pub(crate) async fn switch_authority(
        &self,
        gate: &ModuleGenerationGate,
        expected: AuthorityRecoveryContext,
        switch_budget: Duration,
    ) -> Result<()> {
        self.ensure_ready().await?;
        self.advance_to_switching().await?;
        let deadline = tokio::time::Instant::now() + switch_budget;
        let mut fence = self.fence.close_and_drain(switch_budget).await?;
        let attempt = async {
            let watermark = self.source.capture_watermark().await?;
            let authority_context = AuthorityRecoveryContext {
                source_watermark: watermark,
                ..expected.clone()
            };
            for table_name in self.prepared_tables().await? {
                loop {
                    if tokio::time::Instant::now() >= deadline {
                        bail!("migration switch budget expired while applying the final delta");
                    }
                    let progress = self.catch_up_through(&table_name, watermark).await?;
                    if progress.applied_through >= watermark {
                        break;
                    }
                }
                let proofs = self
                    .target
                    .verified_chunks(self.source.migration_id(), &table_name)
                    .await?;
                for proof in proofs {
                    if tokio::time::Instant::now() >= deadline {
                        bail!("migration switch budget expired while verifying final chunks");
                    }
                    if self.verify_chunk(&table_name, proof.range.clone()).await?
                        != ChunkVerification::Verified
                    {
                        bail!("migration chunk changed during final authority verification");
                    }
                }
            }
            let manifest = ModuleAuthorityManifest {
                migration_id: authority_context.migration_id.clone(),
                installation_id: authority_context.installation_id.clone(),
                source_identity: authority_context.source_identity.clone(),
                module: self.module,
                authority: ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: self.authority_generation.saturating_add(1),
                },
                source_watermark: authority_context.source_watermark,
                target_identity: authority_context.target_identity.clone(),
            };
            self.metadata
                .prepare_authority_context(&authority_context)
                .await?;
            fence.commit(manifest.authority.generation).await?;
            self.metadata
                .save_source_fence(&SourceFenceRecord {
                    migration_id: self.source.migration_id().to_owned(),
                    module: self.module,
                    source_identity: self.source.source_identity().to_owned(),
                    generation: manifest.authority.generation,
                    fenced: true,
                })
                .await?;
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .unwrap_or_default();
            gate.switch_authority(
                self.module,
                manifest,
                authority_context,
                remaining,
                self.metadata.clone(),
            )
            .await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        match attempt {
            Ok(()) => {
                fence.finalize();
                self.save_status(MigrationState::Postgres, None).await
            }
            Err(error) => {
                if fence.prepared_generation.is_none() {
                    fence.reopen().await;
                    return Err(error);
                }
                match self.recover_authority(gate, &expected).await {
                    Ok(()) => match self.metadata.load_authority_manifest(self.module).await? {
                        AuthorityRecoveryRecord::Manifest(_) => {
                            fence.finalize();
                            return Ok(());
                        }
                        AuthorityRecoveryRecord::NeverMigrated { .. } => fence.reopen().await,
                        AuthorityRecoveryRecord::Indeterminate => drop(fence),
                    },
                    Err(recovery_error) => {
                        drop(fence);
                        return Err(error.context(format!(
                            "authority recovery after switch failure failed: {recovery_error}"
                        )));
                    }
                }
                Err(error)
            }
        }
    }

    pub(crate) async fn recover_authority(
        &self,
        gate: &ModuleGenerationGate,
        expected: &AuthorityRecoveryContext,
    ) -> Result<()> {
        let record = self.metadata.load_authority_manifest(self.module).await?;
        let recovery_expected = match &record {
            AuthorityRecoveryRecord::NeverMigrated {
                source_watermark, ..
            } => AuthorityRecoveryContext {
                source_watermark: *source_watermark,
                ..expected.clone()
            },
            AuthorityRecoveryRecord::Manifest(manifest) => AuthorityRecoveryContext {
                source_watermark: manifest.source_watermark,
                ..expected.clone()
            },
            AuthorityRecoveryRecord::Indeterminate => expected.clone(),
        };
        gate.recover_authority(self.metadata.as_ref(), &recovery_expected)
            .await?;
        let resolved = self.metadata.load_authority_manifest(self.module).await?;
        match resolved {
            AuthorityRecoveryRecord::NeverMigrated { .. } => {
                self.fence
                    .recover(BackendKind::Sqlite, self.authority_generation)
                    .await?;
                self.metadata
                    .save_source_fence(&SourceFenceRecord {
                        migration_id: self.source.migration_id().to_owned(),
                        module: self.module,
                        source_identity: self.source.source_identity().to_owned(),
                        generation: self.authority_generation,
                        fenced: false,
                    })
                    .await?;
                self.save_status(MigrationState::CatchingUp, None).await
            }
            AuthorityRecoveryRecord::Manifest(manifest) => {
                self.fence
                    .recover(BackendKind::Postgres, manifest.authority.generation)
                    .await?;
                self.metadata
                    .save_source_fence(&SourceFenceRecord {
                        migration_id: manifest.migration_id,
                        module: manifest.module,
                        source_identity: manifest.source_identity,
                        generation: manifest.authority.generation,
                        fenced: true,
                    })
                    .await?;
                self.save_status(MigrationState::Postgres, None).await
            }
            AuthorityRecoveryRecord::Indeterminate => {
                bail!("authority recovery is indeterminate; source writes remain fenced")
            }
        }
    }

    async fn ensure_ready(&self) -> Result<()> {
        let watermark = self.source.capture_watermark().await?;
        for table_name in self.prepared_tables().await? {
            let plan = self
                .target
                .load_plan(self.source.migration_id(), &table_name)
                .await?
                .ok_or_else(|| anyhow!("migration plan is missing for {table_name}"))?;
            let checkpoint = self
                .target
                .load_checkpoint(self.source.migration_id(), &table_name)
                .await?
                .ok_or_else(|| anyhow!("migration checkpoint is missing for {table_name}"))?;
            if !checkpoint.complete {
                bail!("migration table is not completely copied: {table_name}");
            }
            if checkpoint.applied_through > watermark {
                bail!("migration checkpoint is ahead of the source watermark: {table_name}");
            }
            if checkpoint.applied_through < watermark {
                bail!("migration table has unapplied source changes: {table_name}");
            }
            let proofs = self
                .target
                .verified_chunks(self.source.migration_id(), &table_name)
                .await?;
            let current_lower = self.source.table_lower_bound(&table_name).await?;
            let current_upper = self.source.table_upper_bound(&table_name).await?;
            if proofs.is_empty() && plan.scan_upper_bound.is_some() {
                bail!("migration table has no verified chunks: {table_name}");
            }
            for proof in &proofs {
                if proof.migration_id != self.source.migration_id()
                    || proof.table_name != table_name
                    || proof.range.first_key > proof.range.last_key
                    || plan
                        .scan_lower_bound
                        .as_ref()
                        .is_some_and(|lower| proof.range.last_key < *lower)
                    || current_lower
                        .as_ref()
                        .is_some_and(|lower| proof.range.last_key < *lower)
                    || current_upper
                        .as_ref()
                        .is_some_and(|upper| proof.range.first_key > *upper)
                {
                    bail!("migration table has an invalid verified chunk: {table_name}");
                }
            }
            if let (Some(first_key), Some(last_key)) = (current_lower, current_upper) {
                let source_rows = self
                    .source
                    .rows_in_range(&table_name, &ChunkRange::new(first_key, last_key)?)
                    .await?;
                if source_rows.iter().any(|row| {
                    !proofs.iter().any(|proof| {
                        proof.range.first_key <= row.stable_key
                            && proof.range.last_key >= row.stable_key
                    })
                }) {
                    bail!("migration table has uncovered rows in verified chunks: {table_name}");
                }
            }
            if proofs
                .iter()
                .any(|proof| proof.source_watermark != watermark)
            {
                bail!("migration table has stale verified chunks: {table_name}");
            }
        }
        Ok(())
    }

    async fn prepared_tables(&self) -> Result<Vec<String>> {
        let mut tables = BTreeSet::new();
        for table in self.metadata_tables().await? {
            tables.insert(table);
        }
        if tables.is_empty() {
            bail!("migration has no prepared tables");
        }
        Ok(tables.into_iter().collect())
    }

    async fn metadata_tables(&self) -> Result<Vec<String>> {
        // The target is the durable source of table plans. Implementations expose plans through
        // this bounded probe; callers never scan the source catalog during startup.
        self.target.known_tables(self.source.migration_id()).await
    }

    async fn persist_checkpoint(&self, checkpoint: &EngineCheckpoint) -> Result<()> {
        let contract =
            checkpoint.as_contract(self.source.source_identity(), self.authority_generation);
        self.metadata.save_checkpoint(&contract).await
    }

    async fn progress_for(&self, checkpoint: &EngineCheckpoint) -> Result<MigrationProgress> {
        let pending = self
            .source
            .changes_after(checkpoint.applied_through, self.limits.copy.max_rows)
            .await?
            .len();
        Ok(MigrationProgress {
            table_name: checkpoint.table_name.clone(),
            copied_rows: checkpoint.copied_rows,
            copied_bytes: checkpoint.copied_bytes,
            cursor: checkpoint.cursor.clone(),
            applied_through: checkpoint.applied_through,
            pending_changes: pending,
            complete: checkpoint.complete,
        })
    }

    async fn save_status(&self, state: MigrationState, hold_reason: Option<String>) -> Result<()> {
        let now = epoch_ms();
        let mut status = self
            .metadata
            .load_status(self.module)
            .await?
            .unwrap_or(MigrationStatus {
                migration_id: self.source.migration_id().to_owned(),
                installation_id: self.installation_id.clone(),
                module: self.module,
                state: MigrationState::Pending,
                source_identity: self.source.source_identity().to_owned(),
                target_identity: self.target_identity.clone(),
                authority_generation: self.authority_generation,
                capacity_bytes: DEFAULT_MAX_CAPTURE_BYTES,
                held_bytes: 0,
                hold_reason: None,
                updated_at_epoch_ms: now,
            });
        if status.state != state {
            status.hold_reason = hold_reason;
            status.transition_to(state, now)?;
        } else {
            status.updated_at_epoch_ms = now;
            status.hold_reason = hold_reason;
            status.validate()?;
        }
        self.metadata.save_status(&status).await
    }

    async fn advance_to_switching(&self) -> Result<()> {
        let current = self
            .metadata
            .load_status(self.module)
            .await?
            .map(|status| status.state)
            .unwrap_or(MigrationState::Pending);
        let path = match current {
            MigrationState::Pending => vec![
                MigrationState::Preparing,
                MigrationState::Copying,
                MigrationState::CatchingUp,
                MigrationState::Verifying,
                MigrationState::Ready,
                MigrationState::Switching,
            ],
            MigrationState::Preparing => vec![
                MigrationState::Copying,
                MigrationState::CatchingUp,
                MigrationState::Verifying,
                MigrationState::Ready,
                MigrationState::Switching,
            ],
            MigrationState::Copying => vec![
                MigrationState::CatchingUp,
                MigrationState::Verifying,
                MigrationState::Ready,
                MigrationState::Switching,
            ],
            MigrationState::CatchingUp => vec![
                MigrationState::Verifying,
                MigrationState::Ready,
                MigrationState::Switching,
            ],
            MigrationState::Verifying => {
                vec![MigrationState::Ready, MigrationState::Switching]
            }
            MigrationState::Ready => vec![MigrationState::Switching],
            MigrationState::Switching => Vec::new(),
            MigrationState::Paused
            | MigrationState::Blocked
            | MigrationState::Recovering
            | MigrationState::Failed
            | MigrationState::Postgres => {
                bail!("migration status cannot enter authority switching from {current:?}")
            }
        };
        for state in path {
            self.save_status(state, None).await?;
        }
        Ok(())
    }
}

fn normalized_rows_digest(rows: &[VersionedRow]) -> String {
    let mut rows = rows.to_vec();
    rows.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
    let mut digest = Sha256::new();
    for row in rows {
        digest.update((row.stable_key.len() as u64).to_be_bytes());
        digest.update(row.stable_key.as_bytes());
        digest.update(row.source_version.to_be_bytes());
        digest.update((row.payload.len() as u64).to_be_bytes());
        digest.update(row.payload);
    }
    encode_hex(&digest.finalize())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn epoch_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub(crate) struct InMemoryMigrationSource {
    migration_id: String,
    source_identity: String,
    rows: Arc<Mutex<BTreeMap<String, BTreeMap<String, VersionedRow>>>>,
    capture: VersionCapture,
}

impl InMemoryMigrationSource {
    pub(crate) fn new(migration_id: impl Into<String>, source_identity: impl Into<String>) -> Self {
        let migration_id = migration_id.into();
        let source_identity = source_identity.into();
        Self {
            capture: VersionCapture::new(migration_id.clone(), source_identity.clone()),
            migration_id,
            source_identity,
            rows: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub(crate) async fn write(
        &self,
        fence: &SourceWriteFence,
        changes: Vec<ChangeInput>,
    ) -> Result<i64> {
        let lease = fence.begin_write().await?;
        let version = self.capture.append(changes.clone()).await?;
        let mut rows = self.rows.lock().await;
        for change in changes {
            let table = rows.entry(change.table_name.clone()).or_default();
            match change.kind {
                ChangeKind::Insert | ChangeKind::Update => {
                    table.insert(
                        change.stable_key.clone(),
                        VersionedRow {
                            stable_key: change.stable_key,
                            source_version: version,
                            payload: change.payload.unwrap_or_default(),
                        },
                    );
                }
                ChangeKind::Delete => {
                    table.remove(&change.stable_key);
                }
                ChangeKind::Replace => {
                    if let Some(previous_key) = change.previous_key {
                        table.remove(&previous_key);
                    }
                    table.insert(
                        change.stable_key.clone(),
                        VersionedRow {
                            stable_key: change.stable_key,
                            source_version: version,
                            payload: change.payload.unwrap_or_default(),
                        },
                    );
                }
            }
        }
        lease.complete();
        Ok(version)
    }

    pub(crate) async fn capture(&self, changes: Vec<ChangeInput>) -> Result<i64> {
        self.capture_transaction(changes).await
    }

    async fn capture_transaction(&self, changes: Vec<ChangeInput>) -> Result<i64> {
        self.capture.append(changes).await
    }

    pub(crate) fn capture_log(&self) -> VersionCapture {
        self.capture.clone()
    }
}

#[async_trait]
impl MigrationSource for InMemoryMigrationSource {
    async fn capture_watermark(&self) -> Result<i64> {
        self.capture.watermark().await
    }

    async fn table_lower_bound(&self, table_name: &str) -> Result<Option<String>> {
        Ok(self
            .rows
            .lock()
            .await
            .get(table_name)
            .and_then(|table| table.keys().next().cloned()))
    }

    async fn table_upper_bound(&self, table_name: &str) -> Result<Option<String>> {
        Ok(self
            .rows
            .lock()
            .await
            .get(table_name)
            .and_then(|table| table.keys().next_back().cloned()))
    }

    async fn scan_page(
        &self,
        table_name: &str,
        cursor: Option<&str>,
        upper_bound: Option<&str>,
        limits: CopyLimits,
    ) -> Result<SourcePage> {
        let rows = self.rows.lock().await;
        let table = rows.get(table_name);
        let mut page = Vec::new();
        let mut bytes = 0_usize;
        if let Some(table) = table {
            for row in table.values().filter(|row| {
                cursor.is_none_or(|cursor| row.stable_key.as_str() > cursor)
                    && upper_bound.is_none_or(|bound| row.stable_key.as_str() <= bound)
            }) {
                let row_bytes = row.payload.len();
                if !page.is_empty()
                    && (page.len() >= limits.max_rows
                        || bytes.saturating_add(row_bytes) > limits.max_bytes)
                {
                    break;
                }
                page.push(row.clone());
                bytes = bytes.saturating_add(row_bytes);
            }
        }
        let next_cursor = page.last().map(|row| row.stable_key.clone());
        let complete =
            next_cursor.as_deref() == upper_bound || page.is_empty() || upper_bound.is_none();
        Ok(SourcePage {
            rows: page,
            next_cursor,
            complete,
        })
    }

    async fn rows_in_range(
        &self,
        table_name: &str,
        range: &ChunkRange,
    ) -> Result<Vec<VersionedRow>> {
        Ok(self
            .rows
            .lock()
            .await
            .get(table_name)
            .map(|table| {
                table
                    .values()
                    .filter(|row| {
                        row.stable_key >= range.first_key && row.stable_key <= range.last_key
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn changes_after(&self, after: i64, limit: usize) -> Result<Vec<CapturedChange>> {
        self.capture.changes_after(after, limit).await
    }

    async fn changes_through(
        &self,
        after: i64,
        through: i64,
        limit: usize,
    ) -> Result<Vec<CapturedChange>> {
        self.capture.changes_through(after, through, limit).await
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
    tables: BTreeMap<String, BTreeMap<String, VersionedRow>>,
    tombstones: BTreeMap<(String, String), i64>,
    proofs: BTreeMap<(String, String, String, String), ChunkProof>,
    fail_next_operation: bool,
}

impl InMemoryMigrationTarget {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn fail_next_operation(&self) {
        self.state.lock().await.fail_next_operation = true;
    }

    pub(crate) async fn row(&self, table_name: &str, stable_key: &str) -> Option<VersionedRow> {
        self.state
            .lock()
            .await
            .tables
            .get(table_name)
            .and_then(|table| table.get(stable_key).cloned())
    }

    async fn take_failure(state: &mut InMemoryTargetState) -> Result<()> {
        if state.fail_next_operation {
            state.fail_next_operation = false;
            bail!("injected target commit failure");
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
            .values()
            .filter(|plan| plan.migration_id == migration_id)
            .map(|plan| plan.table_name.clone())
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
        let mut state = self.state.lock().await;
        Self::take_failure(&mut state).await?;
        state.plans.insert(
            (plan.migration_id.clone(), plan.table_name.clone()),
            plan.clone(),
        );
        let checkpoint = EngineCheckpoint {
            migration_id: plan.migration_id.clone(),
            table_name: plan.table_name.clone(),
            cursor: None,
            scan_upper_bound: plan.scan_upper_bound.clone(),
            capture_start: plan.capture_start,
            captured_through: plan.capture_start,
            applied_through: plan.capture_start,
            copied_rows: 0,
            copied_bytes: 0,
            complete: false,
        };
        state.checkpoints.insert(
            (plan.migration_id.clone(), plan.table_name.clone()),
            checkpoint,
        );
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
        let mut state = self.state.lock().await;
        Self::take_failure(&mut state).await?;
        for row in &page.rows {
            let tombstone = state
                .tombstones
                .get(&(plan.table_name.clone(), row.stable_key.clone()))
                .copied();
            if tombstone.is_none_or(|version| version < row.source_version) {
                let table = state.tables.entry(plan.table_name.clone()).or_default();
                let should_replace = table
                    .get(&row.stable_key)
                    .is_none_or(|existing| existing.source_version <= row.source_version);
                if should_replace {
                    table.insert(row.stable_key.clone(), row.clone());
                }
            }
        }
        state.checkpoints.insert(
            (plan.migration_id.clone(), plan.table_name.clone()),
            checkpoint.clone(),
        );
        Ok(())
    }

    async fn apply_change_batch(
        &self,
        migration_id: &str,
        changes: &[CapturedChange],
        checkpoint: &EngineCheckpoint,
    ) -> Result<()> {
        let mut state = self.state.lock().await;
        Self::take_failure(&mut state).await?;
        for change in changes {
            match change.kind {
                ChangeKind::Insert | ChangeKind::Update => {
                    let key = (change.table_name.clone(), change.stable_key.clone());
                    let tombstone = state.tombstones.get(&key).copied();
                    let current_version = state
                        .tables
                        .get(&change.table_name)
                        .and_then(|table| table.get(&change.stable_key))
                        .map(|row| row.source_version);
                    if tombstone.is_none_or(|version| version <= change.source_version)
                        && current_version.is_none_or(|version| version <= change.source_version)
                    {
                        let table = state.tables.entry(change.table_name.clone()).or_default();
                        table.insert(
                            change.stable_key.clone(),
                            VersionedRow {
                                stable_key: change.stable_key.clone(),
                                source_version: change.source_version,
                                payload: change.payload.clone().unwrap_or_default(),
                            },
                        );
                    }
                }
                ChangeKind::Delete => {
                    let key = (change.table_name.clone(), change.stable_key.clone());
                    let tombstone = state.tombstones.get(&key).copied();
                    let current_version = state
                        .tables
                        .get(&change.table_name)
                        .and_then(|table| table.get(&change.stable_key))
                        .map(|row| row.source_version);
                    if tombstone.is_none_or(|version| version <= change.source_version)
                        && current_version.is_none_or(|version| version <= change.source_version)
                    {
                        if let Some(table) = state.tables.get_mut(&change.table_name) {
                            table.remove(&change.stable_key);
                        }
                        state
                            .tombstones
                            .entry(key)
                            .and_modify(|version| *version = (*version).max(change.source_version))
                            .or_insert(change.source_version);
                    }
                }
                ChangeKind::Replace => {
                    if let Some(previous_key) = &change.previous_key {
                        let key = (change.table_name.clone(), previous_key.clone());
                        let tombstone = state.tombstones.get(&key).copied();
                        let current_version = state
                            .tables
                            .get(&change.table_name)
                            .and_then(|table| table.get(previous_key))
                            .map(|row| row.source_version);
                        if tombstone.is_none_or(|version| version <= change.source_version)
                            && current_version
                                .is_none_or(|version| version <= change.source_version)
                        {
                            if let Some(table) = state.tables.get_mut(&change.table_name) {
                                table.remove(previous_key);
                            }
                            state
                                .tombstones
                                .entry(key)
                                .and_modify(|version| {
                                    *version = (*version).max(change.source_version)
                                })
                                .or_insert(change.source_version);
                        }
                    }
                    let key = (change.table_name.clone(), change.stable_key.clone());
                    let tombstone = state.tombstones.get(&key).copied();
                    let current_version = state
                        .tables
                        .get(&change.table_name)
                        .and_then(|table| table.get(&change.stable_key))
                        .map(|row| row.source_version);
                    let should_replace = tombstone
                        .is_none_or(|version| version <= change.source_version)
                        && current_version.is_none_or(|version| version <= change.source_version);
                    if should_replace {
                        state
                            .tables
                            .entry(change.table_name.clone())
                            .or_default()
                            .insert(
                                change.stable_key.clone(),
                                VersionedRow {
                                    stable_key: change.stable_key.clone(),
                                    source_version: change.source_version,
                                    payload: change.payload.clone().unwrap_or_default(),
                                },
                            );
                    }
                }
            }
        }
        state.checkpoints.insert(
            (migration_id.to_owned(), checkpoint.table_name.clone()),
            checkpoint.clone(),
        );
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
            .tables
            .get(table_name)
            .map(|table| {
                table
                    .values()
                    .filter(|row| {
                        row.stable_key.as_str() >= range.first_key.as_str()
                            && row.stable_key.as_str() <= range.last_key.as_str()
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn save_chunk_proof(&self, proof: &ChunkProof) -> Result<()> {
        let mut state = self.state.lock().await;
        Self::take_failure(&mut state).await?;
        state.proofs.insert(
            (
                proof.migration_id.clone(),
                proof.table_name.clone(),
                proof.range.first_key.clone(),
                proof.range.last_key.clone(),
            ),
            proof.clone(),
        );
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
            .proofs
            .values()
            .filter(|proof| proof.migration_id == migration_id && proof.table_name == table_name)
            .cloned()
            .collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileMigrationState {
    schema_version: u32,
    authority: Option<ModuleAuthorityManifest>,
    no_cutover: Option<AuthorityRecoveryContext>,
    statuses: Vec<MigrationStatus>,
    checkpoints: Vec<MigrationCheckpoint>,
    outbox: Vec<MigrationOutboxItem>,
    reservations: Vec<MigrationReservation>,
    source_fence: Option<SourceFenceRecord>,
}

const FILE_STATE_SCHEMA_VERSION: u32 = 1;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(1);
static FILE_STORE_LOCKS: OnceLock<StdMutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

fn file_store_lock(path: &Path) -> Arc<Mutex<()>> {
    let locks = FILE_STORE_LOCKS.get_or_init(|| StdMutex::new(BTreeMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_owned(), Arc::downgrade(&lock));
    lock
}

#[derive(Debug, Clone)]
pub(crate) struct FileMigrationStore {
    path: Arc<PathBuf>,
    initial_context: AuthorityRecoveryContext,
    lock: Arc<Mutex<()>>,
}

impl FileMigrationStore {
    pub(crate) fn new(path: impl Into<PathBuf>, initial_context: AuthorityRecoveryContext) -> Self {
        let path = path.into();
        Self {
            lock: file_store_lock(&path),
            path: Arc::new(path),
            initial_context,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.path.as_ref()
    }

    async fn read_state(&self) -> Result<FileMigrationState> {
        let _lock = self.lock.lock().await;
        self.read_state_unlocked().await
    }

    async fn read_state_unlocked(&self) -> Result<FileMigrationState> {
        if !self.path.exists() {
            return Ok(self.default_state());
        }
        let path = self.path.clone();
        let state = tokio::task::spawn_blocking(move || read_file_state(&path))
            .await
            .context("file migration state task panicked")??;
        self.validate_state(&state)?;
        Ok(state)
    }

    async fn update_state<Value, F>(&self, update: F) -> Result<Value>
    where
        F: FnOnce(&mut FileMigrationState) -> Result<Value>,
    {
        let lock = Arc::clone(&self.lock).lock_owned().await;
        let mut state = self.read_state_unlocked().await?;
        let value = update(&mut state)?;
        let bytes = serde_json::to_vec_pretty(&state).context("serialize migration state")?;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || atomic_write(&path, &bytes))
            .await
            .context("file migration state task panicked")??;
        Ok(value)
    }

    fn default_state(&self) -> FileMigrationState {
        FileMigrationState {
            schema_version: FILE_STATE_SCHEMA_VERSION,
            authority: None,
            no_cutover: Some(self.initial_context.clone()),
            statuses: Vec::new(),
            checkpoints: Vec::new(),
            outbox: Vec::new(),
            reservations: Vec::new(),
            source_fence: None,
        }
    }

    fn validate_status_context(&self, status: &MigrationStatus) -> Result<()> {
        if status.migration_id != self.initial_context.migration_id
            || status.installation_id != self.initial_context.installation_id
            || status.module != self.initial_context.module
            || status.source_identity != self.initial_context.source_identity
            || status.target_identity != self.initial_context.target_identity
        {
            bail!("migration status belongs to another migration context");
        }
        Ok(())
    }

    fn validate_checkpoint_context(&self, checkpoint: &MigrationCheckpoint) -> Result<()> {
        if checkpoint.migration_id != self.initial_context.migration_id
            || checkpoint.source_identity != self.initial_context.source_identity
        {
            bail!("migration checkpoint belongs to another migration context");
        }
        Ok(())
    }

    fn validate_outbox_context(&self, item: &MigrationOutboxItem) -> Result<()> {
        if item.migration_id != self.initial_context.migration_id
            || item.source_identity != self.initial_context.source_identity
            || item.module != self.initial_context.module
        {
            bail!("migration outbox item belongs to another migration context");
        }
        Ok(())
    }

    fn validate_reservation_context(&self, reservation: &MigrationReservation) -> Result<()> {
        if reservation.migration_id != self.initial_context.migration_id
            || reservation.source_identity != self.initial_context.source_identity
            || reservation.module != self.initial_context.module
        {
            bail!("migration reservation belongs to another migration context");
        }
        Ok(())
    }

    fn validate_fence_context(&self, fence: &SourceFenceRecord) -> Result<()> {
        if fence.migration_id != self.initial_context.migration_id
            || fence.module != self.initial_context.module
            || fence.source_identity != self.initial_context.source_identity
            || fence.generation == 0
        {
            bail!("source fence belongs to another migration context");
        }
        Ok(())
    }

    fn validate_state(&self, state: &FileMigrationState) -> Result<()> {
        if let Some(manifest) = state.authority.as_ref()
            && (!context_identity_matches_manifest(&self.initial_context, manifest)
                || manifest.source_watermark < self.initial_context.source_watermark)
        {
            bail!("stored authority manifest belongs to another migration context");
        }
        if let Some(context) = state.no_cutover.as_ref()
            && (!context_identity_matches(context, &self.initial_context)
                || context.source_watermark < self.initial_context.source_watermark)
        {
            bail!("stored no-cutover marker belongs to another migration context");
        }
        if let Some(fence) = state.source_fence.as_ref() {
            self.validate_fence_context(fence)?;
        }
        for status in &state.statuses {
            status.validate()?;
            self.validate_status_context(status)?;
        }
        for checkpoint in &state.checkpoints {
            checkpoint.validate()?;
            self.validate_checkpoint_context(checkpoint)?;
        }
        for item in &state.outbox {
            item.validate()?;
            self.validate_outbox_context(item)?;
        }
        for reservation in &state.reservations {
            reservation.validate()?;
            self.validate_reservation_context(reservation)?;
        }
        Ok(())
    }
}

fn read_file_state(path: &Path) -> Result<FileMigrationState> {
    if !path.exists() {
        return Ok(FileMigrationState {
            schema_version: FILE_STATE_SCHEMA_VERSION,
            authority: None,
            no_cutover: None,
            statuses: Vec::new(),
            checkpoints: Vec::new(),
            outbox: Vec::new(),
            reservations: Vec::new(),
            source_fence: None,
        });
    }
    let bytes =
        fs::read(path).with_context(|| format!("read migration authority {}", path.display()))?;
    let state: FileMigrationState = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse migration authority {}", path.display()))?;
    if state.schema_version != FILE_STATE_SCHEMA_VERSION {
        bail!(
            "unsupported migration authority schema version {}",
            state.schema_version
        );
    }
    Ok(state)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("migration authority path has no parent"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create authority directory {}", parent.display()))?;
    let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{suffix}", std::process::id()));
    let mut file = File::create(&temporary)
        .with_context(|| format!("create temporary authority {}", temporary.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write temporary authority {}", temporary.display()))?;
    file.sync_all()
        .with_context(|| format!("sync temporary authority {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("publish migration authority {}", path.display()))?;
    File::open(parent)
        .with_context(|| format!("open authority directory {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("sync authority directory {}", parent.display()))?;
    Ok(())
}

fn recovery_record(state: &FileMigrationState, module: StorageModule) -> AuthorityRecoveryRecord {
    if let Some(manifest) = state
        .authority
        .clone()
        .filter(|manifest| manifest.module == module)
    {
        AuthorityRecoveryRecord::Manifest(manifest)
    } else if let Some(context) = state
        .no_cutover
        .clone()
        .filter(|context| context.module == module)
    {
        AuthorityRecoveryRecord::NeverMigrated {
            migration_id: context.migration_id,
            installation_id: context.installation_id,
            module: context.module,
            source_identity: context.source_identity,
            target_identity: context.target_identity,
            source_watermark: context.source_watermark,
        }
    } else {
        AuthorityRecoveryRecord::Indeterminate
    }
}

fn context_matches_manifest(
    expected: &AuthorityRecoveryContext,
    manifest: &ModuleAuthorityManifest,
) -> bool {
    context_identity_matches_manifest(expected, manifest)
        && expected.source_watermark == manifest.source_watermark
}

fn context_identity_matches(
    left: &AuthorityRecoveryContext,
    right: &AuthorityRecoveryContext,
) -> bool {
    left.migration_id == right.migration_id
        && left.installation_id == right.installation_id
        && left.module == right.module
        && left.source_identity == right.source_identity
        && left.target_identity == right.target_identity
}

fn context_identity_matches_manifest(
    expected: &AuthorityRecoveryContext,
    manifest: &ModuleAuthorityManifest,
) -> bool {
    expected.migration_id == manifest.migration_id
        && expected.installation_id == manifest.installation_id
        && expected.module == manifest.module
        && expected.source_identity == manifest.source_identity
        && expected.target_identity == manifest.target_identity
}

#[async_trait]
impl AuthorityManifestStore for FileMigrationStore {
    async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord> {
        let state = self.read_state().await?;
        Ok(recovery_record(&state, module))
    }

    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
        expected: &AuthorityRecoveryContext,
    ) -> Result<AuthorityPublication> {
        self.update_state(|state| {
            if !context_matches_manifest(expected, manifest) {
                return Ok(AuthorityPublication::Conflict);
            }
            if let Some(current) = state.authority.as_ref() {
                return Ok(if current == manifest {
                    AuthorityPublication::Committed
                } else {
                    AuthorityPublication::Conflict
                });
            }
            if state.no_cutover.as_ref() != Some(expected) {
                return Ok(AuthorityPublication::Conflict);
            }
            state.authority = Some(manifest.clone());
            state.no_cutover = None;
            Ok(AuthorityPublication::Committed)
        })
        .await
    }
}

#[async_trait]
impl MigrationMetadataStore for FileMigrationStore {
    async fn load_status(&self, module: StorageModule) -> Result<Option<MigrationStatus>> {
        Ok(self
            .read_state()
            .await?
            .statuses
            .into_iter()
            .find(|status| status.module == module))
    }

    async fn save_status(&self, status: &MigrationStatus) -> Result<()> {
        status.validate()?;
        self.validate_status_context(status)?;
        self.update_state(|state| {
            state
                .statuses
                .retain(|existing| existing.module != status.module);
            state.statuses.push(status.clone());
            Ok(())
        })
        .await
    }

    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<MigrationCheckpoint>> {
        Ok(self
            .read_state()
            .await?
            .checkpoints
            .into_iter()
            .find(|checkpoint| {
                checkpoint.migration_id == migration_id && checkpoint.table_name == table_name
            }))
    }

    async fn save_checkpoint(&self, checkpoint: &MigrationCheckpoint) -> Result<()> {
        checkpoint.validate()?;
        self.validate_checkpoint_context(checkpoint)?;
        self.update_state(|state| {
            if let Some(existing) = state.checkpoints.iter().find(|existing| {
                existing.migration_id == checkpoint.migration_id
                    && existing.table_name == checkpoint.table_name
            }) {
                if checkpoint.captured_through < existing.captured_through
                    || checkpoint.applied_through < existing.applied_through
                    || (existing.complete && !checkpoint.complete)
                {
                    bail!("migration checkpoint regresses durable progress");
                }
            }
            state.checkpoints.retain(|existing| {
                existing.migration_id != checkpoint.migration_id
                    || existing.table_name != checkpoint.table_name
            });
            state.checkpoints.push(checkpoint.clone());
            Ok(())
        })
        .await
    }

    async fn enqueue_outbox(&self, item: &MigrationOutboxItem) -> Result<bool> {
        item.validate()?;
        self.validate_outbox_context(item)?;
        self.update_state(|state| {
            if state
                .outbox
                .iter()
                .any(|existing| existing.outbox_id == item.outbox_id)
            {
                return Ok(false);
            }
            state.outbox.push(item.clone());
            Ok(true)
        })
        .await
    }

    async fn acknowledge_outbox(&self, outbox_id: &str, applied_through: i64) -> Result<()> {
        if applied_through < 0 {
            bail!("outbox acknowledgement watermark must not be negative");
        }
        self.update_state(|state| {
            if let Some(item) = state.outbox.iter().find(|item| item.outbox_id == outbox_id) {
                if item.source_version > applied_through {
                    bail!("outbox acknowledgement is below the item source version");
                }
            }
            state.outbox.retain(|item| item.outbox_id != outbox_id);
            Ok(())
        })
        .await
    }

    async fn reserve_capacity(&self, reservation: &MigrationReservation) -> Result<bool> {
        reservation.validate()?;
        self.validate_reservation_context(reservation)?;
        self.update_state(|state| {
            if state
                .reservations
                .iter()
                .any(|existing| existing.reservation_id == reservation.reservation_id)
            {
                return Ok(false);
            }
            state.reservations.push(reservation.clone());
            Ok(true)
        })
        .await
    }

    async fn release_capacity(&self, reservation_id: &str) -> Result<()> {
        self.update_state(|state| {
            state
                .reservations
                .retain(|item| item.reservation_id != reservation_id);
            Ok(())
        })
        .await
    }
}

#[async_trait]
impl MigrationFenceStore for FileMigrationStore {
    async fn load_source_fence(&self, migration_id: &str) -> Result<Option<SourceFenceRecord>> {
        let fence = self
            .read_state()
            .await?
            .source_fence
            .filter(|fence| fence.migration_id == migration_id);
        if let Some(fence) = fence.as_ref() {
            self.validate_fence_context(fence)?;
        }
        Ok(fence)
    }

    async fn save_source_fence(&self, fence: &SourceFenceRecord) -> Result<()> {
        self.validate_fence_context(fence)?;
        self.update_state(|state| {
            state.source_fence = Some(fence.clone());
            Ok(())
        })
        .await
    }

    async fn prepare_authority_context(&self, expected: &AuthorityRecoveryContext) -> Result<()> {
        if expected.source_watermark < 0
            || !context_identity_matches(expected, &self.initial_context)
        {
            bail!("authority context belongs to another migration context");
        }
        self.update_state(|state| {
            if state.authority.is_some() {
                bail!("module authority has already been published");
            }
            let current = state
                .no_cutover
                .as_ref()
                .ok_or_else(|| anyhow!("migration authority context is indeterminate"))?;
            if !context_identity_matches(current, expected)
                || expected.source_watermark < current.source_watermark
            {
                bail!("authority context does not advance the durable no-cutover marker");
            }
            state.no_cutover = Some(expected.clone());
            Ok(())
        })
        .await
    }
}
