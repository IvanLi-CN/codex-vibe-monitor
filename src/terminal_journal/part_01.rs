pub(crate) const TERMINAL_JOURNAL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
const SYSTEM_TASK_QUARANTINE_COMPACTION_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_MAX_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_SYNC_INTERVAL: Duration = Duration::from_millis(20);
const TERMINAL_JOURNAL_REPLAY_MAX_BYTES: usize = 4 * 1024 * 1024;
const TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES: usize = 4 * 1024 * 1024;
const TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES: usize = 1024;

type ShutdownRecoveryKey = (String, String, bool);
type LoadedShutdownTerminalRecovery = (
    HashMap<ShutdownRecoveryKey, ProxyCaptureRecord>,
    Vec<BatchedTerminalInvocationWrite>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalJournalDurabilityMode {
    Journal,
    MemoryOverflow,
}

impl TerminalJournalDurabilityMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Journal => "journal",
            Self::MemoryOverflow => "memory_overflow",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TerminalJournalAppendOutcome {
    pub(crate) durability_mode: TerminalJournalDurabilityMode,
    pub(crate) sequence: Option<u64>,
    pub(crate) pending_records: usize,
    pub(crate) pending_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TerminalJournalStats {
    pub(crate) pending_records: usize,
    pub(crate) pending_bytes: u64,
    pub(crate) segment_count: usize,
    pub(crate) replay_count: usize,
    pub(crate) checkpoint_lag_ms: u64,
    pub(crate) overflowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalTerminalRecord {
    sequence: u64,
    raw_capture: bool,
    #[serde(default)]
    capture_elapsed_ms: Option<u64>,
    record: ProxyCaptureRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalLine {
    #[serde(default)]
    entry: Option<JournalTerminalRecord>,
    #[serde(default)]
    acknowledged_sequence: Option<u64>,
    checksum: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ShutdownTerminalRecoveryLine {
    invoke_id: String,
    occurred_at: String,
    raw_capture: bool,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    record: Option<ProxyCaptureRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SystemTaskRecoveryLine {
    run_id: i64,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default)]
    task_kind: String,
    #[serde(default)]
    trigger_kind: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    finished_at: String,
    #[serde(default)]
    duration_ms: i64,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug)]
struct JournalSegment {
    path: PathBuf,
    first_sequence: u64,
    last_sequence: u64,
    bytes: u64,
    sequences: HashSet<u64>,
    acknowledged: HashSet<u64>,
    current: bool,
}

impl JournalSegment {
    fn is_fully_acknowledged(&self) -> bool {
        self.sequences.is_empty() || self.sequences.len() == self.acknowledged.len()
    }
}

#[derive(Debug)]
struct LoadedJournalSegment {
    segment: JournalSegment,
    entries: Vec<JournalEntryMetadata>,
    acknowledgement_sequences: HashSet<u64>,
}

#[derive(Debug)]
struct JournalEntryMetadata {
    sequence: u64,
    invoke_id: String,
    occurred_at: String,
    raw_capture: bool,
}

#[derive(Debug)]
struct JournalReplayCursor {
    first_sequence: u64,
    byte_offset: u64,
}

#[derive(Debug)]
pub(crate) struct TerminalJournal {
    directory: PathBuf,
    current_file: File,
    segments: BTreeMap<u64, JournalSegment>,
    pending_by_key: HashMap<(String, String, bool), Vec<u64>>,
    next_sequence: u64,
    pending_bytes: u64,
    replay_segments: VecDeque<JournalReplayCursor>,
    replay_count: usize,
    deferred_writes: VecDeque<BatchedTerminalInvocationWrite>,
    replay_blocked: bool,
    shutdown_recovery_pending: HashMap<ShutdownRecoveryKey, ProxyCaptureRecord>,
    system_task_quarantine_ids: HashSet<i64>,
    replayed_system_task_ids: HashSet<i64>,
    deferred_system_task_finishes: VecDeque<BatchedSystemTaskFinish>,
    last_sync_at: Instant,
    last_checkpoint_at: Instant,
    overflowed: bool,
    sync_failed: bool,
}
