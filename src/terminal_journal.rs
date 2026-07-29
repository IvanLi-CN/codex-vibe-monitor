use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::{BatchedTerminalInvocationWrite, ProxyCaptureRecord};

pub(crate) const TERMINAL_JOURNAL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_MAX_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_SYNC_INTERVAL: Duration = Duration::from_millis(20);

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
        !self.sequences.is_empty() && self.sequences.len() == self.acknowledged.len()
    }
}

#[derive(Debug)]
pub(crate) struct TerminalJournal {
    directory: PathBuf,
    current_file: File,
    segments: BTreeMap<u64, JournalSegment>,
    pending_by_key: HashMap<(String, String, bool), Vec<u64>>,
    next_sequence: u64,
    pending_bytes: u64,
    replay: Vec<JournalTerminalRecord>,
    deferred_writes: VecDeque<BatchedTerminalInvocationWrite>,
    last_sync_at: Instant,
    last_checkpoint_at: Instant,
    overflowed: bool,
}

impl TerminalJournal {
    pub(crate) fn open(database_path: &Path) -> Result<Self> {
        let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
        let directory = parent.join("terminal_journal");
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
            .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .collect::<Vec<_>>();
        paths.sort();

        let mut segments = BTreeMap::new();
        let mut pending_by_key = HashMap::new();
        let mut replay = Vec::new();
        let mut next_sequence = 1_u64;
        let mut pending_bytes = 0_u64;
        for path in paths {
            let (segment, entries) = load_segment(&path)?;
            if let Some(segment) = segment {
                next_sequence = next_sequence.max(segment.last_sequence.saturating_add(1));
                pending_bytes = pending_bytes.saturating_add(segment.bytes);
                for entry in &entries {
                    pending_by_key
                        .entry(entry_key(entry))
                        .or_insert_with(Vec::new)
                        .push(entry.sequence);
                }
                replay.extend(
                    entries
                        .into_iter()
                        .filter(|entry| !segment.acknowledged.contains(&entry.sequence)),
                );
                segments.insert(segment.first_sequence, segment);
            }
        }

        let current_start = segments
            .last_key_value()
            .map(|(_, segment)| segment.first_sequence)
            .unwrap_or(next_sequence);
        if let Some(segment) = segments.get_mut(&current_start) {
            segment.current = true;
        } else {
            let path = segment_path(&directory, current_start);
            let file = open_append_file(&path)?;
            segments.insert(
                current_start,
                JournalSegment {
                    path,
                    first_sequence: current_start,
                    last_sequence: current_start.saturating_sub(1),
                    bytes: 0,
                    sequences: HashSet::new(),
                    acknowledged: HashSet::new(),
                    current: true,
                },
            );
            return Ok(Self {
                directory,
                current_file: file,
                segments,
                pending_by_key,
                next_sequence,
                pending_bytes,
                replay,
                deferred_writes: VecDeque::new(),
                last_sync_at: Instant::now(),
                last_checkpoint_at: Instant::now(),
                overflowed: pending_bytes >= TERMINAL_JOURNAL_MAX_BYTES,
            });
        }

        let current_path = segments
            .get(&current_start)
            .expect("current terminal journal segment exists")
            .path
            .clone();
        Ok(Self {
            directory,
            current_file: open_append_file(&current_path)?,
            segments,
            pending_by_key,
            next_sequence,
            pending_bytes,
            replay,
            deferred_writes: VecDeque::new(),
            last_sync_at: Instant::now(),
            last_checkpoint_at: Instant::now(),
            overflowed: pending_bytes >= TERMINAL_JOURNAL_MAX_BYTES,
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
                Some(started.elapsed().as_millis() as u64)
            }
            Err(err) => {
                warn!(error = %err, "terminal journal group commit failed");
                self.overflowed = true;
                None
            }
        }
    }

    pub(crate) fn force_sync(&mut self) -> Result<()> {
        self.current_file
            .sync_data()
            .context("failed to sync terminal journal")?;
        self.last_sync_at = Instant::now();
        Ok(())
    }

    pub(crate) fn acknowledge(&mut self, invoke_id: &str, occurred_at: &str, raw_capture: bool) {
        let key = (invoke_id.to_string(), occurred_at.to_string(), raw_capture);
        let Some(sequences) = self.pending_by_key.remove(&key) else {
            return;
        };
        for sequence in sequences {
            let Ok(encoded) = encode_acknowledgement_line(sequence) else {
                warn!(
                    sequence,
                    "terminal journal acknowledgement serialization failed"
                );
                continue;
            };
            if let Err(err) = self.current_file.write_all(&encoded) {
                warn!(error = %err, sequence, "terminal journal acknowledgement append failed");
                continue;
            }
            for segment in self.segments.values_mut() {
                if segment.sequences.contains(&sequence) {
                    segment.acknowledged.insert(sequence);
                    segment.bytes = segment.bytes.saturating_add(encoded.len() as u64);
                    self.pending_bytes = self.pending_bytes.saturating_add(encoded.len() as u64);
                    break;
                }
            }
        }
        self.last_checkpoint_at = Instant::now();
        self.remove_fully_acknowledged_segments();
        if self.pending_bytes < TERMINAL_JOURNAL_MAX_BYTES * 3 / 5 {
            self.overflowed = false;
        }
    }

    pub(crate) fn take_replay(&mut self) -> Vec<BatchedTerminalInvocationWrite> {
        let replay = std::mem::take(&mut self.replay);
        replay
            .into_iter()
            .map(|entry| BatchedTerminalInvocationWrite {
                record: entry.record,
                capture_started: entry.capture_elapsed_ms.and_then(|elapsed_ms| {
                    Instant::now().checked_sub(Duration::from_millis(elapsed_ms))
                }),
                raw_capture: entry.raw_capture,
                dashboard_terminal_sequence: None,
            })
            .collect()
    }

    pub(crate) fn defer_write(&mut self, write: BatchedTerminalInvocationWrite) -> bool {
        self.deferred_writes.push_back(write);
        true
    }

    pub(crate) fn queue_replay_for_dispatch(&mut self) {
        let replay = self.take_replay();
        self.deferred_writes.extend(replay);
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
            replay_count: self.replay.len(),
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

fn segment_path(directory: &Path, first_sequence: u64) -> PathBuf {
    directory.join(format!("terminal-{first_sequence:020}.jsonl"))
}

fn open_append_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open terminal journal segment {}", path.display()))
}

fn load_segment(path: &Path) -> Result<(Option<JournalSegment>, Vec<JournalTerminalRecord>)> {
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
            (Some(entry), None) => entries.push(entry),
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
    let Some(first) = entries.first() else {
        return Ok((None, Vec::new()));
    };
    let last_sequence = entries
        .last()
        .map(|entry| entry.sequence)
        .unwrap_or(first.sequence);
    let sequences = entries
        .iter()
        .map(|entry| entry.sequence)
        .collect::<HashSet<_>>();
    Ok((
        Some(JournalSegment {
            path: path.to_path_buf(),
            first_sequence: first.sequence,
            last_sequence,
            bytes: valid_bytes as u64,
            sequences,
            acknowledged,
            current: false,
        }),
        entries,
    ))
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

fn entry_key(entry: &JournalTerminalRecord) -> (String, String, bool) {
    (
        entry.record.invoke_id.clone(),
        entry.record.occurred_at.clone(),
        entry.raw_capture,
    )
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
        let evidence_count = fs::read_dir(root.join("terminal_journal"))
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
}
