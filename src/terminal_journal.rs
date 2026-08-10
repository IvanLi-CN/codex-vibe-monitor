use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::{
    BatchedSystemTaskFinish, BatchedTerminalInvocationWrite, ProxyCaptureRecord,
    api_invocation_from_runtime_record, startup_backfill_tasks_for_terminal,
};

pub(crate) const TERMINAL_JOURNAL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_MAX_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_SYNC_INTERVAL: Duration = Duration::from_millis(20);
const TERMINAL_JOURNAL_REPLAY_MAX_BYTES: usize = 4 * 1024 * 1024;
const TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES: usize = 4 * 1024 * 1024;
const TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES: usize = 1024;
const SYSTEM_TASK_QUARANTINE_INDEX_MAX_BYTES: usize = 4 * 1024 * 1024;
const SYSTEM_TASK_QUARANTINE_INDEX_MAX_LINES: usize = 4096;

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
    system_task_quarantine_ids: HashSet<i64>,
    last_sync_at: Instant,
    last_checkpoint_at: Instant,
    overflowed: bool,
    sync_failed: bool,
}

impl TerminalJournal {
    pub(crate) fn quarantine(
        &mut self,
        terminal: &BatchedTerminalInvocationWrite,
        error: &str,
    ) -> Result<()> {
        self.quarantine_terminals(std::slice::from_ref(&terminal), error)
    }

    pub(crate) fn quarantine_terminals(
        &mut self,
        terminals: &[&BatchedTerminalInvocationWrite],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct QuarantineEntry<'a> {
            invoke_id: &'a str,
            occurred_at: &'a str,
            raw_capture: bool,
            error: &'a str,
            record: &'a ProxyCaptureRecord,
        }

        let path = self.directory.join("quarantine.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open terminal quarantine {}", path.display()))?;
        for terminal in terminals {
            let entry = QuarantineEntry {
                invoke_id: &terminal.record.invoke_id,
                occurred_at: &terminal.record.occurred_at,
                raw_capture: terminal.raw_capture,
                error,
                record: &terminal.record,
            };
            serde_json::to_writer(&mut file, &entry)
                .context("failed to encode terminal quarantine")?;
            file.write_all(b"\n")
                .context("failed to append terminal quarantine delimiter")?;
        }
        if !terminals.is_empty() {
            file.sync_data()
                .context("failed to sync terminal quarantine")?;
        }
        Ok(())
    }

    pub(crate) fn quarantine_system_task_finish(
        &mut self,
        finish: &BatchedSystemTaskFinish,
        error: &str,
    ) -> Result<()> {
        self.quarantine_system_task_finishes(std::slice::from_ref(&finish), error)
    }

    pub(crate) fn quarantine_system_task_finishes(
        &mut self,
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct QuarantineEntry<'a> {
            run_id: i64,
            task_kind: &'static str,
            trigger_kind: &'a str,
            status: &'static str,
            summary: &'a Option<String>,
            detail: &'a Option<String>,
            finished_at: &'a str,
            duration_ms: i64,
            error: &'a str,
        }

        let path = self.directory.join("system-task-quarantine.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open system-task quarantine {}", path.display()))?;
        let mut appended = false;
        let mut newly_indexed_ids = Vec::new();
        for finish in finishes {
            if !self.system_task_quarantine_ids.insert(finish.run_id) {
                continue;
            }
            newly_indexed_ids.push(finish.run_id);
            let entry = QuarantineEntry {
                run_id: finish.run_id,
                task_kind: finish.task_kind.as_str(),
                trigger_kind: &finish.trigger_kind,
                status: finish.status.as_str(),
                summary: &finish.summary,
                detail: &finish.detail,
                finished_at: &finish.finished_at,
                duration_ms: finish.duration_ms,
                error,
            };
            if let Err(err) = serde_json::to_writer(&mut file, &entry) {
                for run_id in newly_indexed_ids {
                    self.system_task_quarantine_ids.remove(&run_id);
                }
                return Err(err).context("failed to append system-task quarantine");
            }
            if let Err(err) = file.write_all(b"\n") {
                for run_id in newly_indexed_ids {
                    self.system_task_quarantine_ids.remove(&run_id);
                }
                return Err(err).context("failed to append system-task quarantine delimiter");
            }
            appended = true;
        }
        if appended && let Err(err) = file.sync_data() {
            for run_id in newly_indexed_ids {
                self.system_task_quarantine_ids.remove(&run_id);
            }
            return Err(err).context("failed to sync system-task quarantine");
        }
        Ok(())
    }

    pub(crate) fn quarantine_shutdown_batch_at_database_path(
        database_path: &Path,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct TerminalEntry<'a> {
            invoke_id: &'a str,
            occurred_at: &'a str,
            raw_capture: bool,
            error: &'a str,
            record: &'a ProxyCaptureRecord,
        }
        #[derive(Serialize)]
        struct SystemTaskEntry<'a> {
            run_id: i64,
            task_kind: &'static str,
            trigger_kind: &'a str,
            status: &'static str,
            summary: &'a Option<String>,
            detail: &'a Option<String>,
            finished_at: &'a str,
            duration_ms: i64,
            error: &'a str,
        }

        let directory = terminal_journal_directory(database_path);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create independent terminal recovery directory {}",
                directory.display()
            )
        })?;
        if !terminals.is_empty() {
            let path = directory.join("shutdown-terminal-quarantine.jsonl");
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| {
                    format!("failed to open terminal recovery sink {}", path.display())
                })?;
            for terminal in terminals {
                serde_json::to_writer(
                    &mut file,
                    &TerminalEntry {
                        invoke_id: &terminal.record.invoke_id,
                        occurred_at: &terminal.record.occurred_at,
                        raw_capture: terminal.raw_capture,
                        error,
                        record: &terminal.record,
                    },
                )
                .context("failed to encode terminal recovery entry")?;
                file.write_all(b"\n")
                    .context("failed to append terminal recovery delimiter")?;
            }
            file.sync_data()
                .context("failed to sync terminal recovery sink")?;
        }
        if !finishes.is_empty() {
            let path = directory.join("shutdown-system-task-quarantine.jsonl");
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| {
                    format!(
                        "failed to open system-task recovery sink {}",
                        path.display()
                    )
                })?;
            for finish in finishes {
                serde_json::to_writer(
                    &mut file,
                    &SystemTaskEntry {
                        run_id: finish.run_id,
                        task_kind: finish.task_kind.as_str(),
                        trigger_kind: &finish.trigger_kind,
                        status: finish.status.as_str(),
                        summary: &finish.summary,
                        detail: &finish.detail,
                        finished_at: &finish.finished_at,
                        duration_ms: finish.duration_ms,
                        error,
                    },
                )
                .context("failed to encode system-task recovery entry")?;
                file.write_all(b"\n")
                    .context("failed to append system-task recovery delimiter")?;
            }
            file.sync_data()
                .context("failed to sync system-task recovery sink")?;
        }
        Ok(())
    }

    pub(crate) fn open(database_path: &Path) -> Result<Self> {
        let directory = terminal_journal_directory(database_path);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create terminal journal directory {}",
                directory.display()
            )
        })?;

        let mut paths = fs::read_dir(&directory)
            .with_context(|| {
                format!(
                    "failed to list terminal journal directory {}",
                    directory.display()
                )
            })?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "jsonl")
                    && path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(|stem| stem.starts_with("terminal-"))
            })
            .collect::<Vec<_>>();
        paths.sort();

        let mut segments = BTreeMap::new();
        let mut loaded_segments = Vec::new();
        let mut acknowledged_sequences = HashSet::new();
        let mut next_sequence = 1_u64;
        let mut pending_bytes = 0_u64;
        for path in paths {
            let loaded = load_segment(&path)?;
            next_sequence = next_sequence.max(loaded.segment.last_sequence.saturating_add(1));
            pending_bytes = pending_bytes.saturating_add(loaded.segment.bytes);
            acknowledged_sequences.extend(loaded.acknowledgement_sequences);
            loaded_segments.push((loaded.segment.first_sequence, loaded.entries));
            segments.insert(loaded.segment.first_sequence, loaded.segment);
        }

        let current_start = segments
            .last_key_value()
            .map(|(_, segment)| segment.first_sequence)
            .unwrap_or(next_sequence);
        segments.entry(current_start).or_insert_with(|| {
            let path = segment_path(&directory, current_start);
            JournalSegment {
                path,
                first_sequence: current_start,
                last_sequence: current_start.saturating_sub(1),
                bytes: 0,
                sequences: HashSet::new(),
                acknowledged: HashSet::new(),
                current: true,
            }
        });
        for segment in segments.values_mut() {
            segment.acknowledged.extend(
                acknowledged_sequences
                    .intersection(&segment.sequences)
                    .copied(),
            );
        }
        let mut pending_by_key = HashMap::new();
        let mut replay_segments = VecDeque::new();
        let mut replay_count = 0_usize;
        for (first_sequence, entries) in loaded_segments {
            let mut segment_has_replay = false;
            for entry in entries {
                if acknowledged_sequences.contains(&entry.sequence) {
                    continue;
                }
                pending_by_key
                    .entry((entry.invoke_id, entry.occurred_at, entry.raw_capture))
                    .or_insert_with(Vec::new)
                    .push(entry.sequence);
                replay_count = replay_count.saturating_add(1);
                segment_has_replay = true;
            }
            if segment_has_replay {
                replay_segments.push_back(JournalReplayCursor {
                    first_sequence,
                    byte_offset: 0,
                });
            }
        }
        segments
            .get_mut(&current_start)
            .expect("current terminal journal segment exists")
            .current = true;

        let current_path = segments
            .get(&current_start)
            .expect("current terminal journal segment exists")
            .path
            .clone();
        let system_task_quarantine_ids =
            load_system_task_quarantine_ids(&directory.join("system-task-quarantine.jsonl"));
        Ok(Self {
            directory,
            current_file: open_append_file(&current_path)?,
            segments,
            pending_by_key,
            next_sequence,
            pending_bytes,
            replay_segments,
            replay_count,
            deferred_writes: VecDeque::new(),
            replay_blocked: false,
            system_task_quarantine_ids,
            last_sync_at: Instant::now(),
            last_checkpoint_at: Instant::now(),
            overflowed: pending_bytes >= TERMINAL_JOURNAL_MAX_BYTES,
            sync_failed: false,
        })
    }

    pub(crate) fn append(
        &mut self,
        record: &ProxyCaptureRecord,
        raw_capture: bool,
        capture_started: Option<Instant>,
    ) -> TerminalJournalAppendOutcome {
        let key = (
            record.invoke_id.clone(),
            record.occurred_at.clone(),
            raw_capture,
        );
        if self.sync_failed {
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if self.pending_by_key.contains_key(&key) {
            return self.append_outcome(TerminalJournalDurabilityMode::Journal, None);
        }
        let entry = JournalTerminalRecord {
            sequence: self.next_sequence,
            raw_capture,
            capture_elapsed_ms: capture_started
                .map(|started| started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            record: record.clone(),
        };
        let Ok(encoded) = encode_entry_line(&entry) else {
            warn!(invoke_id = %record.invoke_id, occurred_at = %record.occurred_at, "terminal journal serialization failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        };
        let encoded_len = encoded.len() as u64;
        if self.pending_bytes.saturating_add(encoded_len) > TERMINAL_JOURNAL_MAX_BYTES {
            self.overflowed = true;
            warn!(
                journal_pending_bytes = self.pending_bytes,
                journal_max_bytes = TERMINAL_JOURNAL_MAX_BYTES,
                invoke_id = %record.invoke_id,
                occurred_at = %record.occurred_at,
                "terminal journal reached capacity; using memory fallback"
            );
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if let Err(err) = self.rotate_if_needed(encoded_len) {
            warn!(error = %err, "terminal journal rotation failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if let Err(err) = self.current_file.write_all(&encoded) {
            warn!(error = %err, "terminal journal append failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        let current_start = self
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current terminal journal segment exists");
        let segment = self
            .segments
            .get_mut(&current_start)
            .expect("current terminal journal segment exists");
        segment.last_sequence = entry.sequence;
        segment.bytes = segment.bytes.saturating_add(encoded_len);
        segment.sequences.insert(entry.sequence);
        self.pending_by_key.insert(key, vec![entry.sequence]);
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.pending_bytes = self.pending_bytes.saturating_add(encoded_len);
        self.append_outcome(TerminalJournalDurabilityMode::Journal, Some(entry.sequence))
    }

    pub(crate) fn sync_if_due(&mut self) -> Option<u64> {
        if self.last_sync_at.elapsed() < TERMINAL_JOURNAL_SYNC_INTERVAL {
            return None;
        }
        let started = Instant::now();
        match self.current_file.sync_data() {
            Ok(()) => {
                self.last_sync_at = Instant::now();
                self.sync_failed = false;
                Some(started.elapsed().as_millis() as u64)
            }
            Err(err) => {
                warn!(error = %err, "terminal journal group commit failed");
                self.overflowed = true;
                self.sync_failed = true;
                None
            }
        }
    }

    pub(crate) fn force_sync(&mut self) -> Result<()> {
        match self.current_file.sync_data() {
            Ok(()) => {
                self.last_sync_at = Instant::now();
                self.sync_failed = false;
                Ok(())
            }
            Err(err) => {
                self.overflowed = true;
                self.sync_failed = true;
                Err(err).context("failed to sync terminal journal")
            }
        }
    }

    pub(crate) fn acknowledge(&mut self, invoke_id: &str, occurred_at: &str, raw_capture: bool) {
        let key = (invoke_id.to_string(), occurred_at.to_string(), raw_capture);
        let Some(sequences) = self.pending_by_key.get(&key).cloned() else {
            return;
        };
        let mut pending_sequences = Vec::new();
        let mut acknowledged_replay_count = 0_usize;
        for sequence in sequences {
            let Ok(encoded) = encode_acknowledgement_line(sequence) else {
                warn!(
                    sequence,
                    "terminal journal acknowledgement serialization failed"
                );
                pending_sequences.push(sequence);
                continue;
            };
            if let Err(err) = self.current_file.write_all(&encoded) {
                warn!(error = %err, sequence, "terminal journal acknowledgement append failed");
                pending_sequences.push(sequence);
                continue;
            }
            let acknowledgement_target = self.segments.iter().find_map(|(start, segment)| {
                segment.sequences.contains(&sequence).then_some(*start)
            });
            if let Some(start) = acknowledgement_target {
                self.segments
                    .get_mut(&start)
                    .expect("terminal journal acknowledgement target exists")
                    .acknowledged
                    .insert(sequence);
            }
            let current_start = self
                .segments
                .iter()
                .find_map(|(start, segment)| segment.current.then_some(*start))
                .expect("current terminal journal segment exists");
            let updated_current_bytes = self
                .segments
                .get(&current_start)
                .expect("current terminal journal segment exists")
                .bytes
                .saturating_add(encoded.len() as u64);
            self.segments
                .get_mut(&current_start)
                .expect("current terminal journal segment exists")
                .bytes = updated_current_bytes;
            self.pending_bytes = self.pending_bytes.saturating_add(encoded.len() as u64);
            acknowledged_replay_count = acknowledged_replay_count.saturating_add(1);
        }
        if pending_sequences.is_empty() {
            self.pending_by_key.remove(&key);
        } else {
            self.pending_by_key.insert(key, pending_sequences);
        }
        self.last_checkpoint_at = Instant::now();
        self.replay_count = self.replay_count.saturating_sub(acknowledged_replay_count);
        self.remove_fully_acknowledged_segments();
        if !self.sync_failed && self.pending_bytes < TERMINAL_JOURNAL_MAX_BYTES * 3 / 5 {
            self.overflowed = false;
        }
    }

    pub(crate) fn take_replay(&mut self) -> Vec<BatchedTerminalInvocationWrite> {
        self.take_replay_chunk(usize::MAX, usize::MAX)
    }

    pub(crate) fn queue_replay_for_dispatch(&mut self, max_writes: usize) {
        if max_writes == 0 || self.deferred_writes.len() >= max_writes {
            return;
        }
        let available = max_writes.saturating_sub(self.deferred_writes.len());
        let replay = self.take_replay_chunk(available, TERMINAL_JOURNAL_REPLAY_MAX_BYTES);
        self.deferred_writes.extend(replay);
    }

    fn take_replay_chunk(
        &mut self,
        max_writes: usize,
        max_bytes: usize,
    ) -> Vec<BatchedTerminalInvocationWrite> {
        if self.replay_blocked {
            return Vec::new();
        }
        let mut writes = Vec::new();
        let mut estimated_bytes = 0_usize;
        let mut scanned_bytes = 0_usize;
        let mut scanned_lines = 0_usize;
        while writes.len() < max_writes
            && scanned_bytes < TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES
            && scanned_lines < TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES
        {
            let Some(cursor) = self.replay_segments.front_mut() else {
                break;
            };
            let Some(segment) = self.segments.get(&cursor.first_sequence) else {
                self.replay_segments.pop_front();
                continue;
            };
            let Ok(file) = File::open(&segment.path) else {
                break;
            };
            let mut reader = BufReader::new(file);
            if reader.seek(SeekFrom::Start(cursor.byte_offset)).is_err() {
                break;
            }
            let mut reached_end = false;
            let mut produced = None;
            loop {
                let stream_start = cursor.byte_offset;
                let (mut parsed, budget_exhausted) = {
                    let mut budget_reader = ReplayBudgetReader {
                        reader: &mut reader,
                        remaining: TERMINAL_JOURNAL_REPLAY_MAX_BYTES,
                    };
                    let mut stream = serde_json::Deserializer::from_reader(&mut budget_reader)
                        .into_iter::<JournalLine>();
                    let parsed = stream.next();
                    (parsed, budget_reader.remaining == 0)
                };
                let mut stream_end = match reader.stream_position() {
                    Ok(stream_end) => stream_end,
                    Err(_) => break,
                };
                let mut oversized_record = false;
                if parsed.as_ref().is_some_and(Result::is_err) && budget_exhausted {
                    if reader.seek(SeekFrom::Start(stream_start)).is_err() {
                        break;
                    }
                    parsed = {
                        let mut stream = serde_json::Deserializer::from_reader(&mut reader)
                            .into_iter::<JournalLine>();
                        stream.next()
                    };
                    stream_end = match reader.stream_position() {
                        Ok(stream_end) => stream_end,
                        Err(_) => break,
                    };
                    oversized_record = parsed.as_ref().is_some_and(Result::is_ok);
                }
                if stream_end == stream_start {
                    reached_end = true;
                    break;
                }
                let Some(parsed) = parsed else {
                    cursor.byte_offset = stream_end;
                    reached_end = true;
                    break;
                };
                scanned_bytes = scanned_bytes.saturating_add(
                    stream_end
                        .saturating_sub(stream_start)
                        .min(usize::MAX as u64) as usize,
                );
                scanned_lines = scanned_lines.saturating_add(1);
                let Ok(parsed) = parsed else {
                    cursor.byte_offset = stream_start;
                    self.replay_blocked = true;
                    warn!(
                        replay_offset = stream_start,
                        "terminal journal replay stopped at an unparseable entry; preserving the cursor for recovery"
                    );
                    break;
                };
                cursor.byte_offset = stream_end;
                let Some(entry) = parsed.entry else {
                    if scanned_bytes >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES
                        || scanned_lines >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES
                    {
                        break;
                    }
                    continue;
                };
                if oversized_record {
                    warn!(
                        sequence = entry.sequence,
                        record_bytes = stream_end.saturating_sub(stream_start),
                        replay_batch_bytes = TERMINAL_JOURNAL_REPLAY_MAX_BYTES,
                        "replaying a single terminal journal record larger than the normal batch budget"
                    );
                }
                if segment.acknowledged.contains(&entry.sequence) {
                    if scanned_bytes >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES
                        || scanned_lines >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES
                    {
                        break;
                    }
                    continue;
                }
                let record = entry.record;
                let startup_backfill_tasks = startup_backfill_tasks_for_terminal(
                    &api_invocation_from_runtime_record(&record),
                );
                let write = BatchedTerminalInvocationWrite {
                    capture_started: entry.capture_elapsed_ms.and_then(|elapsed_ms| {
                        Instant::now().checked_sub(Duration::from_millis(elapsed_ms))
                    }),
                    raw_capture: entry.raw_capture,
                    dashboard_terminal_sequence: None,
                    terminal_projection_event_ids: Vec::new(),
                    startup_backfill_tasks,
                    record,
                };
                let write_bytes = write.estimated_memory_bytes();
                if !writes.is_empty() && estimated_bytes.saturating_add(write_bytes) > max_bytes {
                    cursor.byte_offset = stream_start;
                    reached_end = false;
                    break;
                }
                estimated_bytes = estimated_bytes.saturating_add(write_bytes);
                produced = Some(write);
                break;
            }
            if let Some(write) = produced {
                writes.push(write);
                continue;
            }
            if reached_end {
                self.replay_segments.pop_front();
                continue;
            }
            if scanned_bytes >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES
                || scanned_lines >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES
            {
                break;
            }
            break;
        }
        writes
    }

    pub(crate) fn defer_write(&mut self, write: BatchedTerminalInvocationWrite) -> bool {
        self.deferred_writes.push_back(write);
        true
    }

    pub(crate) fn deferred_write_count(&self) -> usize {
        self.deferred_writes.len()
    }

    pub(crate) fn take_deferred_writes(
        &mut self,
        max_writes: usize,
    ) -> Vec<BatchedTerminalInvocationWrite> {
        let count = max_writes.min(self.deferred_writes.len());
        self.deferred_writes.drain(..count).collect()
    }

    pub(crate) fn stats(&self) -> TerminalJournalStats {
        TerminalJournalStats {
            pending_records: self.pending_by_key.len(),
            pending_bytes: self.pending_bytes,
            segment_count: self.segments.len(),
            replay_count: self.replay_count,
            checkpoint_lag_ms: self.last_checkpoint_at.elapsed().as_millis() as u64,
            overflowed: self.overflowed,
        }
    }

    fn append_outcome(
        &self,
        durability_mode: TerminalJournalDurabilityMode,
        sequence: Option<u64>,
    ) -> TerminalJournalAppendOutcome {
        let stats = self.stats();
        TerminalJournalAppendOutcome {
            durability_mode,
            sequence,
            pending_records: stats.pending_records,
            pending_bytes: stats.pending_bytes,
        }
    }

    fn rotate_if_needed(&mut self, next_len: u64) -> Result<()> {
        let current_start = self
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current terminal journal segment exists");
        let should_rotate = self.segments.get(&current_start).is_some_and(|segment| {
            segment.bytes > 0
                && segment.bytes.saturating_add(next_len) > TERMINAL_JOURNAL_SEGMENT_BYTES
        });
        if !should_rotate {
            return Ok(());
        }
        self.current_file
            .sync_data()
            .context("failed to sync terminal journal before rotation")?;
        let start = self.next_sequence;
        let path = segment_path(&self.directory, start);
        let next_file = open_append_file(&path)?;
        self.segments
            .get_mut(&current_start)
            .expect("current terminal journal segment exists")
            .current = false;
        self.current_file = next_file;
        self.segments.insert(
            start,
            JournalSegment {
                path,
                first_sequence: start,
                last_sequence: start.saturating_sub(1),
                bytes: 0,
                sequences: HashSet::new(),
                acknowledged: HashSet::new(),
                current: true,
            },
        );
        self.remove_fully_acknowledged_segments();
        Ok(())
    }

    fn remove_fully_acknowledged_segments(&mut self) {
        let removable = self
            .segments
            .iter()
            .filter_map(|(start, segment)| {
                (!segment.current && segment.is_fully_acknowledged()).then_some(*start)
            })
            .collect::<Vec<_>>();
        for start in removable {
            if let Some(segment) = self.segments.remove(&start) {
                if let Err(err) = fs::remove_file(&segment.path) {
                    warn!(error = %err, path = %segment.path.display(), "failed to remove acknowledged terminal journal segment");
                    self.segments.insert(start, segment);
                } else {
                    self.pending_bytes = self.pending_bytes.saturating_sub(segment.bytes);
                }
            }
        }
    }
}

struct ReplayBudgetReader<'a> {
    reader: &'a mut BufReader<File>,
    remaining: usize,
}

impl Read for ReplayBudgetReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal journal replay record exceeds the bounded input budget",
            ));
        }
        let limit = buffer.len().min(self.remaining);
        let read = self.reader.read(&mut buffer[..limit])?;
        self.remaining = self.remaining.saturating_sub(read);
        Ok(read)
    }
}

fn load_system_task_quarantine_ids(path: &Path) -> HashSet<i64> {
    let Ok(file) = File::open(path) else {
        return HashSet::new();
    };
    let mut reader = BufReader::new(file);
    let mut ids = HashSet::new();
    let mut line = String::new();
    let mut bytes_read = 0_usize;
    for _ in 0..SYSTEM_TASK_QUARANTINE_INDEX_MAX_LINES {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            break;
        };
        if read == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(read);
        if bytes_read > SYSTEM_TASK_QUARANTINE_INDEX_MAX_BYTES {
            warn!(
                path = %path.display(),
                max_bytes = SYSTEM_TASK_QUARANTINE_INDEX_MAX_BYTES,
                "bounded system-task quarantine index reached its startup budget"
            );
            break;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
            && let Some(run_id) = value.get("run_id").and_then(serde_json::Value::as_i64)
        {
            ids.insert(run_id);
        }
    }
    ids
}

fn segment_path(directory: &Path, first_sequence: u64) -> PathBuf {
    directory.join(format!("terminal-{first_sequence:020}.jsonl"))
}

fn terminal_journal_directory(database_path: &Path) -> PathBuf {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let database_id = format!(
        "{:x}",
        Sha256::digest(database_path.as_os_str().as_encoded_bytes())
    );
    parent.join(format!("terminal_journal-{database_id}"))
}

fn open_append_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open terminal journal segment {}", path.display()))
}

fn load_segment(path: &Path) -> Result<LoadedJournalSegment> {
    let raw = fs::read(path)
        .with_context(|| format!("failed to read terminal journal segment {}", path.display()))?;
    let mut entries = Vec::new();
    let mut acknowledged = HashSet::new();
    let mut valid_bytes = 0usize;
    let mut corrupt_tail = false;
    for (line_number, raw_line) in raw.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !raw_line.ends_with(b"\n") {
            corrupt_tail = true;
            break;
        }
        let line = match std::str::from_utf8(&raw_line[..raw_line.len() - 1]) {
            Ok(line) => line,
            Err(_) => {
                corrupt_tail = true;
                break;
            }
        };
        if line.trim().is_empty() {
            valid_bytes += raw_line.len();
            continue;
        }
        let parsed = serde_json::from_str::<JournalLine>(line);
        let Ok(line) = parsed else {
            warn!(path = %path.display(), line_number, "terminal journal has corrupt line; repairing segment tail");
            corrupt_tail = true;
            break;
        };
        if journal_line_checksum(line.entry.as_ref(), line.acknowledged_sequence)? != line.checksum
        {
            warn!(path = %path.display(), line_number, "terminal journal checksum mismatch; repairing segment tail");
            corrupt_tail = true;
            break;
        }
        match (line.entry, line.acknowledged_sequence) {
            (Some(entry), None) => entries.push(JournalEntryMetadata {
                sequence: entry.sequence,
                invoke_id: entry.record.invoke_id,
                occurred_at: entry.record.occurred_at,
                raw_capture: entry.raw_capture,
            }),
            (None, Some(sequence)) => {
                acknowledged.insert(sequence);
            }
            _ => {
                warn!(path = %path.display(), line_number, "terminal journal line has invalid event shape; repairing segment tail");
                corrupt_tail = true;
                break;
            }
        }
        valid_bytes += raw_line.len();
    }
    if corrupt_tail {
        repair_corrupt_segment_tail(path, &raw[..valid_bytes])?;
    }
    let first_sequence = terminal_journal_segment_start(path)?;
    let last_sequence = entries
        .last()
        .map(|entry| entry.sequence)
        .unwrap_or(first_sequence.saturating_sub(1));
    let sequences = entries
        .iter()
        .map(|entry| entry.sequence)
        .collect::<HashSet<_>>();
    Ok(LoadedJournalSegment {
        segment: JournalSegment {
            path: path.to_path_buf(),
            first_sequence,
            last_sequence,
            bytes: valid_bytes as u64,
            sequences,
            acknowledged: HashSet::new(),
            current: false,
        },
        entries,
        acknowledgement_sequences: acknowledged,
    })
}

fn terminal_journal_segment_start(path: &Path) -> Result<u64> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("terminal-"))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "invalid terminal journal segment filename: {}",
                path.display()
            )
        })
}

fn encode_entry_line(entry: &JournalTerminalRecord) -> Result<Vec<u8>> {
    let line = JournalLine {
        entry: Some(entry.clone()),
        acknowledged_sequence: None,
        checksum: journal_line_checksum(Some(entry), None)?,
    };
    let mut bytes = serde_json::to_vec(&line).context("failed to encode terminal journal line")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn encode_acknowledgement_line(sequence: u64) -> Result<Vec<u8>> {
    let line = JournalLine {
        entry: None,
        acknowledged_sequence: Some(sequence),
        checksum: journal_line_checksum(None, Some(sequence))?,
    };
    let mut bytes =
        serde_json::to_vec(&line).context("failed to encode terminal journal acknowledgement")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn journal_line_checksum(
    entry: Option<&JournalTerminalRecord>,
    acknowledged_sequence: Option<u64>,
) -> Result<String> {
    let encoded = serde_json::to_vec(&(entry, acknowledged_sequence))
        .context("failed to checksum terminal journal event")?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn repair_corrupt_segment_tail(path: &Path, valid_prefix: &[u8]) -> Result<()> {
    let evidence_path = path.with_extension(format!(
        "jsonl.corrupt-{}",
        chrono::Utc::now().timestamp_millis()
    ));
    fs::rename(path, &evidence_path).with_context(|| {
        format!(
            "failed to preserve corrupt terminal journal segment {}",
            path.display()
        )
    })?;
    fs::write(path, valid_prefix).with_context(|| {
        format!(
            "failed to rebuild terminal journal segment {} after corruption",
            path.display()
        )
    })?;
    warn!(
        path = %path.display(),
        evidence_path = %evidence_path.display(),
        valid_prefix_bytes = valid_prefix.len(),
        "repaired terminal journal corrupt tail while preserving evidence"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_unacknowledged_record_and_clears_pending_ack() {
        let root = std::env::temp_dir().join(format!("terminal-journal-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record = crate::tests::test_proxy_capture_record("journal-1", "2026-07-29T00:00:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        let capture_started = Instant::now()
            .checked_sub(Duration::from_millis(50))
            .expect("capture start should precede now");
        let outcome = journal.append(&record, false, Some(capture_started));
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
        assert_eq!(outcome.pending_records, 1);
        journal.force_sync().expect("sync journal");
        drop(journal);

        let mut replayed = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(replayed.stats().replay_count, 1);
        let replay = replayed.take_replay();
        assert_eq!(replay.len(), 1);
        let replayed_capture_started = replay[0]
            .capture_started
            .expect("replay should preserve raw capture timing");
        assert!(replayed_capture_started.elapsed() >= Duration::from_millis(50));
        replayed.acknowledge("journal-1", "2026-07-29T00:00:00Z", false);
        replayed.force_sync().expect("sync acknowledgement");
        assert_eq!(replayed.stats().pending_records, 0);
        drop(replayed);

        let replayed_after_ack =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert_eq!(replayed_after_ack.stats().replay_count, 0);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn replay_dispatch_is_bounded_by_the_requested_batch_size() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-replay-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        for index in 0..5 {
            let record = crate::tests::test_proxy_capture_record(
                &format!("journal-replay-{index}"),
                &format!("2026-07-29T00:0{index}:00Z"),
            );
            journal.append(&record, false, None);
        }
        journal.force_sync().expect("sync replay records");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        reopened.queue_replay_for_dispatch(2);
        assert_eq!(reopened.deferred_write_count(), 2);
        let first_batch = reopened.take_deferred_writes(2);
        assert_eq!(first_batch.len(), 2);
        reopened.queue_replay_for_dispatch(2);
        assert_eq!(reopened.deferred_write_count(), 2);
        let second_batch = reopened.take_deferred_writes(2);
        assert_eq!(second_batch.len(), 2);
        reopened.queue_replay_for_dispatch(2);
        assert_eq!(reopened.deferred_write_count(), 1);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn repairs_corrupt_tail_before_accepting_new_records() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-corrupt-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let first =
            crate::tests::test_proxy_capture_record("journal-corrupt-1", "2026-07-29T00:00:00Z");
        let second =
            crate::tests::test_proxy_capture_record("journal-corrupt-2", "2026-07-29T00:01:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&first, false, None);
        journal.force_sync().expect("sync journal");
        let segment_path = journal
            .segments
            .values()
            .find(|segment| segment.current)
            .expect("current segment")
            .path
            .clone();
        drop(journal);
        fs::OpenOptions::new()
            .append(true)
            .open(&segment_path)
            .expect("open journal for corruption")
            .write_all(b"{torn")
            .expect("append corrupt tail");

        let mut repaired = TerminalJournal::open(&database_path).expect("repair journal tail");
        assert_eq!(repaired.stats().replay_count, 1);
        repaired.append(&second, false, None);
        repaired.force_sync().expect("sync repaired journal");
        drop(repaired);

        let reopened = TerminalJournal::open(&database_path).expect("reopen repaired journal");
        assert_eq!(reopened.stats().replay_count, 2);
        let evidence_count = fs::read_dir(terminal_journal_directory(&database_path))
            .expect("read journal directory")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext != "jsonl"))
            .count();
        assert_eq!(evidence_count, 1);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn rotation_open_failure_keeps_existing_segment_current() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-rotation-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        let current_start = journal
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current segment");
        journal
            .segments
            .get_mut(&current_start)
            .expect("current segment")
            .bytes = TERMINAL_JOURNAL_SEGMENT_BYTES;
        journal.directory = root.join("missing-journal-directory");

        assert!(journal.rotate_if_needed(1).is_err());
        assert!(
            journal
                .segments
                .get(&current_start)
                .is_some_and(|segment| segment.current)
        );

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn sync_failure_downgrades_following_appends_to_memory_mode() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-sync-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record =
            crate::tests::test_proxy_capture_record("journal-sync-failed", "2026-07-29T00:00:00Z");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.sync_failed = true;

        let outcome = journal.append(&record, false, None);
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        );
        assert_eq!(outcome.sequence, None);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn acknowledged_restart_key_does_not_block_a_new_terminal_entry() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-rekey-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record =
            crate::tests::test_proxy_capture_record("journal-rekey", "2026-07-29T00:00:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&record, false, None);
        journal.force_sync().expect("sync entry");
        journal.acknowledge(&record.invoke_id, &record.occurred_at, false);
        journal.force_sync().expect("sync acknowledgement");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(reopened.stats().pending_records, 0);
        let outcome = reopened.append(&record, false, None);
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
        assert!(outcome.sequence.is_some());
        reopened.force_sync().expect("sync replacement entry");
        drop(reopened);

        let reopened = TerminalJournal::open(&database_path).expect("reopen replacement entry");
        assert_eq!(reopened.stats().replay_count, 1);
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn restart_applies_acknowledgement_written_in_a_later_segment() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-cross-ack-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let first =
            crate::tests::test_proxy_capture_record("journal-cross-ack-1", "2026-07-29T00:00:00Z");
        let retained =
            crate::tests::test_proxy_capture_record("journal-cross-ack-2", "2026-07-29T00:01:00Z");
        let current =
            crate::tests::test_proxy_capture_record("journal-cross-ack-3", "2026-07-29T00:02:00Z");

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&first, false, None);
        journal.append(&retained, false, None);
        let first_segment = journal
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("first segment");
        journal
            .segments
            .get_mut(&first_segment)
            .expect("first segment")
            .bytes = TERMINAL_JOURNAL_SEGMENT_BYTES;
        journal.append(&current, false, None);
        let current_segment = journal
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current segment");
        let first_segment_bytes = journal
            .segments
            .get(&first_segment)
            .expect("first segment")
            .bytes;
        let current_segment_bytes = journal
            .segments
            .get(&current_segment)
            .expect("current segment")
            .bytes;

        journal.acknowledge(&first.invoke_id, &first.occurred_at, false);
        assert_eq!(
            journal
                .segments
                .get(&first_segment)
                .expect("first segment")
                .bytes,
            first_segment_bytes
        );
        assert!(
            journal
                .segments
                .get(&current_segment)
                .expect("current segment")
                .bytes
                > current_segment_bytes
        );
        journal.force_sync().expect("sync journal");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        let replay = reopened.take_replay();
        let replayed_ids = replay
            .iter()
            .map(|write| write.record.invoke_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(replayed_ids.len(), 2);
        assert!(replayed_ids.contains(retained.invoke_id.as_str()));
        assert!(replayed_ids.contains(current.invoke_id.as_str()));
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn journals_are_isolated_for_databases_in_the_same_directory() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-isolation-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let first_database = root.join("first.db");
        let second_database = root.join("second.db");
        let record =
            crate::tests::test_proxy_capture_record("journal-isolated", "2026-07-29T00:00:00Z");

        let mut first = TerminalJournal::open(&first_database).expect("open first journal");
        first.append(&record, false, None);
        first.force_sync().expect("sync first journal");
        drop(first);

        let second = TerminalJournal::open(&second_database).expect("open second journal");
        assert_eq!(second.stats().replay_count, 0);
        assert_ne!(
            terminal_journal_directory(&first_database),
            terminal_journal_directory(&second_database)
        );
        fs::remove_dir_all(root).expect("remove journal test directory");
    }
}
