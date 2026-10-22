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

mod completion_capacity;
mod segment_recovery;
mod shutdown_recovery;
use completion_capacity::CompletionCapacity;
use segment_recovery::load_segment;
use shutdown_recovery::{
    compact_shutdown_terminal_recovery, load_shutdown_terminal_recovery,
    open_shutdown_recovery_append,
};

use crate::{
    BatchedSystemTaskFinish, BatchedTerminalInvocationWrite, ParsedUsage, ProxyCaptureRecord,
    RawPayloadMeta, RuntimeAttemptSnapshot, StageTimings, SystemTaskKind, SystemTaskStatus,
    api_invocation_from_runtime_record, startup_backfill_tasks_for_terminal,
};

pub(crate) const TERMINAL_JOURNAL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
const SYSTEM_TASK_QUARANTINE_COMPACTION_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_MAX_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const TERMINAL_JOURNAL_SYNC_INTERVAL: Duration = Duration::from_millis(20);
const TERMINAL_JOURNAL_MAX_DEFERRED_WRITES: usize = 4_096;
const TERMINAL_JOURNAL_MAX_DEFERRED_BYTES: usize = 64 * 1024 * 1024;
const TERMINAL_JOURNAL_EVENT_VERSION: u8 = 1;
const TERMINAL_JOURNAL_REPLAY_MAX_BYTES: usize = 4 * 1024 * 1024;
const TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES: usize = 4 * 1024 * 1024;
const TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES: usize = 1024;

type ShutdownRecoveryKey = (String, String, bool);
type ShutdownRecoveryValue = (ProxyCaptureRecord, Vec<RuntimeAttemptSnapshot>);
type LoadedShutdownTerminalRecovery = (
    HashMap<ShutdownRecoveryKey, ShutdownRecoveryValue>,
    Vec<BatchedTerminalInvocationWrite>,
    bool,
    Vec<ShutdownRecoveryKey>,
);

fn journal_safe_terminal_record(record: &ProxyCaptureRecord) -> ProxyCaptureRecord {
    let mut safe = record.clone();
    safe.payload = None;
    safe.raw_response.clear();
    safe.response_body_preview_enabled = safe.resp_raw.path.is_some();
    safe.error_message = None;
    safe
}

fn journal_safe_attempt_snapshot(snapshot: &RuntimeAttemptSnapshot) -> RuntimeAttemptSnapshot {
    let mut safe = snapshot.clone();
    safe.request_summary_json = None;
    safe.error_message = None;
    safe.downstream_error_message = None;
    safe.requester_ip = None;
    safe.sticky_key = None;
    safe.proxy_binding_key_snapshot = None;
    safe.routing_selection_audit_json = None;
    safe.upstream_request_id = None;
    safe.compact_support_reason = None;
    if let Ok(mut route) = url::Url::parse(&safe.upstream_route_key) {
        let _ = route.set_username("");
        let _ = route.set_password(None);
        safe.upstream_route_key = route.to_string();
    }
    safe
}

pub(crate) fn interrupted_terminal_record(
    invoke_id: &str,
    occurred_at: &str,
    attempts: &[RuntimeAttemptSnapshot],
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: attempts
            .first()
            .and_then(|attempt| attempt.request_model.clone()),
        usage: ParsedUsage::default(),
        cost: None,
        cost_breakdown: None,
        cost_estimated: false,
        price_version: None,
        status: crate::INVOCATION_STATUS_INTERRUPTED.to_string(),
        error_message: Some(crate::INVOCATION_INTERRUPTED_MESSAGE.to_string()),
        failure_kind: Some(crate::PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string()),
        payload: None,
        raw_response: String::new(),
        response_body_preview_enabled: false,
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms: 0.0,
            t_req_parse_ms: 0.0,
            t_upstream_connect_ms: 0.0,
            t_upstream_ttfb_ms: 0.0,
            first_token_ms: None,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

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

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TerminalJournalStats {
    pub(crate) pending_records: usize,
    pub(crate) pending_materialization_records: usize,
    pub(crate) pending_bytes: u64,
    #[serde(skip)]
    pub(crate) completion_reserved_bytes: u64,
    pub(crate) segment_count: usize,
    pub(crate) replay_count: usize,
    pub(crate) checkpoint_lag_ms: u64,
    pub(crate) oldest_pending_age_ms: u64,
    pub(crate) overflowed: bool,
    pub(crate) unavailable: bool,
    pub(crate) deferred_records: usize,
    pub(crate) deferred_bytes: usize,
    pub(crate) deferred_rejected: u64,
}

pub(crate) fn terminal_journal_p1_reservation(stats: TerminalJournalStats) -> usize {
    if stats.unavailable {
        stats.deferred_records
    } else {
        stats.replay_count
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalTerminalRecord {
    sequence: u64,
    raw_capture: bool,
    #[serde(default)]
    capture_elapsed_ms: Option<u64>,
    #[serde(default)]
    attempts: Vec<RuntimeAttemptSnapshot>,
    record: ProxyCaptureRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalAttemptStart {
    #[serde(default = "default_journal_event_version")]
    event_version: u8,
    sequence: u64,
    attempt: RuntimeAttemptSnapshot,
}

fn default_journal_event_version() -> u8 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalLine {
    #[serde(default = "default_journal_event_version")]
    event_version: u8,
    #[serde(default)]
    entry: Option<JournalTerminalRecord>,
    #[serde(default)]
    attempt_start: Option<JournalAttemptStart>,
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
    #[serde(default)]
    attempts: Vec<RuntimeAttemptSnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SystemTaskRecoveryLine {
    run_id: i64,
    #[serde(default)]
    acknowledged: bool,
    #[serde(default)]
    task_kind: String,
    #[serde(default)]
    task_key: String,
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
    completion: Option<String>,
    #[serde(default)]
    core_completion: Option<String>,
    #[serde(default)]
    details: Option<serde_json::Value>,
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
    attempt_starts: Vec<JournalAttemptStartMetadata>,
    acknowledgement_sequences: HashSet<u64>,
}

#[derive(Debug)]
struct JournalEntryMetadata {
    sequence: u64,
    invoke_id: String,
    occurred_at: String,
    raw_capture: bool,
    attempt_public_ids: Vec<String>,
}

#[derive(Debug)]
struct JournalAttemptStartMetadata {
    sequence: u64,
    snapshot: RuntimeAttemptSnapshot,
}

#[derive(Debug)]
struct JournalReplayCursor {
    first_sequence: u64,
    byte_offset: u64,
    limit_sequence: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct TerminalJournal {
    directory: PathBuf,
    current_file: File,
    segments: BTreeMap<u64, JournalSegment>,
    pending_by_key: HashMap<(String, String, bool), Vec<u64>>,
    pending_attempt_starts: HashMap<String, u64>,
    completion_capacity: CompletionCapacity,
    dispatch_offsets: HashMap<(String, String, bool), (u64, u64, u64)>,
    next_sequence: u64,
    synced_sequence: u64,
    pending_bytes: u64,
    replay_segments: VecDeque<JournalReplayCursor>,
    replay_count: usize,
    deferred_writes: VecDeque<BatchedTerminalInvocationWrite>,
    deferred_write_bytes: usize,
    deferred_rejected: u64,
    recovery_dispatch_started: bool,
    replay_blocked: bool,
    shutdown_recovery_pending: HashMap<ShutdownRecoveryKey, ShutdownRecoveryValue>,
    shutdown_recovery_order: HashMap<ShutdownRecoveryKey, u64>,
    shutdown_recovery_next_order: u64,
    shutdown_recovery_source_clean: bool,
    shutdown_recovery_bytes: usize,
    pending_since: Option<Instant>,
    system_task_quarantine_ids: HashSet<i64>,
    replayed_system_task_ids: HashSet<i64>,
    system_task_ack_retry_ids: HashSet<i64>,
    deferred_system_task_finishes: VecDeque<BatchedSystemTaskFinish>,
    last_sync_at: Instant,
    last_checkpoint_at: Instant,
    overflowed: bool,
    sync_failed: bool,
    append_failed: bool,
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
        _error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct QuarantineEntry {
            invoke_id: String,
            occurred_at: String,
            raw_capture: bool,
            error: String,
            record: ProxyCaptureRecord,
            attempts: Vec<RuntimeAttemptSnapshot>,
        }

        let path = self.directory.join("quarantine.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open terminal quarantine {}", path.display()))?;
        for terminal in terminals {
            let entry = QuarantineEntry {
                invoke_id: terminal.record.invoke_id.clone(),
                occurred_at: terminal.record.occurred_at.clone(),
                raw_capture: terminal.raw_capture,
                // Quarantine files are durable operational artifacts. Keep the
                // reason categorical so an anyhow chain cannot smuggle payloads,
                // credentials, or response fragments into the journal directory.
                error: "terminal_materialization_failed".to_string(),
                record: journal_safe_terminal_record(&terminal.record),
                attempts: terminal
                    .attempts
                    .iter()
                    .map(journal_safe_attempt_snapshot)
                    .collect(),
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
            task_key: &'a str,
            trigger_kind: &'a str,
            status: &'static str,
            summary: &'a Option<String>,
            detail: &'a Option<String>,
            finished_at: &'a str,
            duration_ms: i64,
            completion: &'a Option<String>,
            core_completion: &'a Option<String>,
            details: &'a Option<serde_json::Value>,
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
        let mut newly_deferred_finishes = Vec::new();
        for finish in finishes {
            if !self.system_task_quarantine_ids.insert(finish.run_id) {
                continue;
            }
            newly_indexed_ids.push(finish.run_id);
            self.replayed_system_task_ids.insert(finish.run_id);
            newly_deferred_finishes.push((*finish).clone());
            let entry = QuarantineEntry {
                run_id: finish.run_id,
                task_kind: finish.task_kind.as_str(),
                task_key: &finish.task_key,
                trigger_kind: &finish.trigger_kind,
                status: finish.status.as_str(),
                summary: &finish.summary,
                detail: &finish.detail,
                finished_at: &finish.finished_at,
                duration_ms: finish.duration_ms,
                completion: &finish.completion,
                core_completion: &finish.core_completion,
                details: &finish.details,
                error,
            };
            if let Err(err) = serde_json::to_writer(&mut file, &entry) {
                for run_id in newly_indexed_ids {
                    self.system_task_quarantine_ids.remove(&run_id);
                    self.replayed_system_task_ids.remove(&run_id);
                }
                return Err(err).context("failed to append system-task quarantine");
            }
            if let Err(err) = file.write_all(b"\n") {
                for run_id in newly_indexed_ids {
                    self.system_task_quarantine_ids.remove(&run_id);
                    self.replayed_system_task_ids.remove(&run_id);
                }
                return Err(err).context("failed to append system-task quarantine delimiter");
            }
            appended = true;
        }
        if appended && let Err(err) = file.sync_data() {
            for run_id in newly_indexed_ids {
                self.system_task_quarantine_ids.remove(&run_id);
                self.replayed_system_task_ids.remove(&run_id);
            }
            return Err(err).context("failed to sync system-task quarantine");
        }
        for finish in newly_deferred_finishes {
            self.deferred_system_task_finishes.push_back(finish);
        }
        Ok(())
    }

    pub(crate) fn take_system_task_finishes(&mut self, max: usize) -> Vec<BatchedSystemTaskFinish> {
        self.deferred_system_task_finishes
            .drain(..self.deferred_system_task_finishes.len().min(max))
            .collect()
    }

    pub(crate) fn remove_deferred_system_task_finishes(&mut self, run_ids: &[i64]) {
        let run_ids = run_ids.iter().copied().collect::<HashSet<_>>();
        self.deferred_system_task_finishes
            .retain(|finish| !run_ids.contains(&finish.run_id));
    }

    pub(crate) fn acknowledge_system_task_finishes(&mut self, run_ids: &[i64]) -> Result<()> {
        let acknowledged = run_ids
            .iter()
            .copied()
            .filter(|run_id| self.replayed_system_task_ids.contains(run_id))
            .collect::<Vec<_>>();
        if acknowledged.is_empty() {
            return Ok(());
        }
        let path = self.directory.join("system-task-quarantine.jsonl");
        append_system_task_acknowledgements(&path, &acknowledged)?;
        let shutdown_path = self.directory.join("shutdown-system-task-quarantine.jsonl");
        if shutdown_path.exists() {
            // Shutdown recovery is read alongside the regular quarantine file. Keep an ACK in
            // that source too, otherwise regular-file compaction can erase the only ACK evidence
            // and replay the same task finish after the next restart.
            append_system_task_acknowledgements(&shutdown_path, &acknowledged)?;
        }
        let acknowledged_ids = acknowledged.into_iter().collect::<HashSet<_>>();
        self.replayed_system_task_ids
            .retain(|run_id| !acknowledged_ids.contains(run_id));
        self.system_task_ack_retry_ids
            .retain(|run_id| !acknowledged_ids.contains(run_id));
        self.deferred_system_task_finishes
            .retain(|finish| !acknowledged_ids.contains(&finish.run_id));
        if let Err(err) = self.compact_system_task_quarantine_if_due() {
            warn!(error = %err, "failed to compact system-task quarantine journal");
        }
        Ok(())
    }

    fn compact_system_task_quarantine_if_due(&mut self) -> Result<()> {
        let path = self.directory.join("system-task-quarantine.jsonl");
        let shutdown_path = self.directory.join("shutdown-system-task-quarantine.jsonl");
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            .max(
                fs::metadata(&shutdown_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0),
            );
        if size < SYSTEM_TASK_QUARANTINE_COMPACTION_BYTES {
            return Ok(());
        }
        // Rebuild from durable recovery files instead of relying on the in-memory deferred queue.
        // Deterministic failures deliberately remove their payload from memory while retaining
        // the journal record for the next restart; compaction must preserve those records too.
        let recovery_paths = [path.clone(), shutdown_path.clone()];
        let recovery_finishes = load_system_task_finishes(&recovery_paths);
        let recovery_ids = recovery_finishes
            .iter()
            .map(|finish| finish.run_id)
            .collect::<HashSet<_>>();

        let temp_path = path.with_extension("jsonl.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)
            .with_context(|| {
                format!(
                    "failed to open system-task quarantine temp file {}",
                    temp_path.display()
                )
            })?;
        for finish in &recovery_finishes {
            serde_json::to_writer(
                &mut file,
                &SystemTaskRecoveryLine {
                    run_id: finish.run_id,
                    acknowledged: false,
                    task_kind: finish.task_kind.as_str().to_string(),
                    task_key: finish.task_key.clone(),
                    trigger_kind: finish.trigger_kind.clone(),
                    status: finish.status.as_str().to_string(),
                    summary: finish.summary.clone(),
                    detail: finish.detail.clone(),
                    finished_at: finish.finished_at.clone(),
                    duration_ms: finish.duration_ms,
                    completion: finish.completion.clone(),
                    core_completion: finish.core_completion.clone(),
                    details: finish.details.clone(),
                    error: None,
                },
            )
            .context("failed to encode compacted system-task quarantine entry")?;
            file.write_all(b"\n")
                .context("failed to append compacted system-task quarantine delimiter")?;
        }
        file.sync_data()
            .context("failed to sync compacted system-task quarantine")?;
        fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "failed to replace system-task quarantine with compacted file {}",
                path.display()
            )
        })?;
        if shutdown_path.exists() {
            let shutdown_temp_path = shutdown_path.with_extension("jsonl.tmp");
            let shutdown_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&shutdown_temp_path)
                .with_context(|| {
                    format!(
                        "failed to open shutdown system-task quarantine temp file {}",
                        shutdown_temp_path.display()
                    )
                })?;
            shutdown_file
                .sync_data()
                .context("failed to sync compacted shutdown system-task quarantine")?;
            fs::rename(&shutdown_temp_path, &shutdown_path).with_context(|| {
                format!(
                    "failed to replace shutdown system-task quarantine with compacted file {}",
                    shutdown_path.display()
                )
            })?;
        }
        self.system_task_quarantine_ids = recovery_ids;
        Ok(())
    }

    pub(crate) fn quarantine_shutdown_batch(
        &mut self,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        let result = Self::quarantine_shutdown_batch_at_directory(
            &self.directory,
            terminals,
            finishes,
            error,
        );
        if result.is_err() {
            // A partial supplemental append must survive any later ACK cleanup.
            self.shutdown_recovery_source_clean = false;
        }
        result
    }

    pub(crate) fn quarantine_shutdown_batch_at_database_path(
        database_path: &Path,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        let directory = terminal_journal_directory(database_path);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create independent terminal recovery directory {}",
                directory.display()
            )
        })?;
        Self::quarantine_shutdown_batch_at_directory(&directory, terminals, finishes, error)
    }

    fn quarantine_shutdown_batch_at_directory(
        directory: &Path,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct SystemTaskEntry<'a> {
            run_id: i64,
            task_kind: &'static str,
            task_key: &'a str,
            trigger_kind: &'a str,
            status: &'static str,
            summary: &'a Option<String>,
            detail: &'a Option<String>,
            finished_at: &'a str,
            duration_ms: i64,
            completion: &'a Option<String>,
            core_completion: &'a Option<String>,
            details: &'a Option<serde_json::Value>,
            error: &'a str,
        }

        fs::create_dir_all(directory).with_context(|| {
            format!(
                "failed to create terminal journal directory {}",
                directory.display()
            )
        })?;
        if !terminals.is_empty() {
            let path = directory.join("shutdown-terminal-quarantine.jsonl");
            let mut file = open_shutdown_recovery_append(&path, true).with_context(|| {
                format!("failed to open terminal recovery sink {}", path.display())
            })?;
            for terminal in terminals {
                let encoded = serde_json::to_vec(&ShutdownTerminalRecoveryLine {
                    invoke_id: terminal.record.invoke_id.clone(),
                    occurred_at: terminal.record.occurred_at.clone(),
                    raw_capture: terminal.raw_capture,
                    acknowledged: false,
                    error: Some("terminal_materialization_failed".to_string()),
                    record: Some(journal_safe_terminal_record(&terminal.record)),
                    attempts: terminal
                        .attempts
                        .iter()
                        .map(journal_safe_attempt_snapshot)
                        .collect(),
                })
                .context("failed to encode terminal recovery entry")?;
                let current_bytes = file
                    .metadata()
                    .context("failed to inspect terminal recovery capacity")?
                    .len();
                let encoded_bytes = encoded.len().saturating_add(1) as u64;
                if current_bytes.saturating_add(encoded_bytes)
                    > TERMINAL_JOURNAL_MAX_DEFERRED_BYTES as u64
                {
                    return Err(anyhow::anyhow!("terminal recovery capacity exhausted"));
                }
                file.write_all(&encoded)
                    .context("failed to append terminal recovery entry")?;
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
                        task_key: &finish.task_key,
                        trigger_kind: &finish.trigger_kind,
                        status: finish.status.as_str(),
                        summary: &finish.summary,
                        detail: &finish.detail,
                        finished_at: &finish.finished_at,
                        duration_ms: finish.duration_ms,
                        completion: &finish.completion,
                        core_completion: &finish.core_completion,
                        details: &finish.details,
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
            loaded_segments.push((
                loaded.segment.first_sequence,
                loaded.entries,
                loaded.attempt_starts,
            ));
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
        let mut pending_attempt_starts = HashMap::new();
        let mut unmatched_attempt_starts = Vec::new();
        let mut replay_segments = VecDeque::new();
        let mut replay_count = 0_usize;
        let mut terminal_attempt_public_ids = HashSet::new();
        for (_, entries, _) in &loaded_segments {
            for entry in entries {
                terminal_attempt_public_ids.extend(entry.attempt_public_ids.iter().cloned());
            }
        }
        for (first_sequence, entries, attempt_starts) in loaded_segments {
            for start in attempt_starts {
                if !acknowledged_sequences.contains(&start.sequence)
                    && !terminal_attempt_public_ids.contains(&start.snapshot.attempt_public_id)
                {
                    pending_attempt_starts
                        .insert(start.snapshot.attempt_public_id.clone(), start.sequence);
                    unmatched_attempt_starts.push(start.snapshot);
                } else if let Some(segment) = segments.get_mut(&first_sequence) {
                    segment.acknowledged.insert(start.sequence);
                }
            }
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
                    limit_sequence: None,
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
        let shutdown_recovery_path = directory.join("shutdown-terminal-quarantine.jsonl");
        let (
            shutdown_recovery_pending,
            recovery_writes,
            shutdown_recovery_source_clean,
            shutdown_recovery_ordered_keys,
        ) = load_shutdown_terminal_recovery(&shutdown_recovery_path);
        let shutdown_recovery_order = shutdown_recovery_ordered_keys
            .iter()
            .enumerate()
            .map(|(order, key)| (key.clone(), order as u64))
            .collect::<HashMap<_, _>>();
        let shutdown_recovery_bytes = shutdown_recovery_pending
            .values()
            .map(|(record, attempts)| {
                record.estimated_memory_bytes()
                    + attempts
                        .iter()
                        .map(RuntimeAttemptSnapshot::estimated_memory_bytes)
                        .sum::<usize>()
            })
            .sum();
        let mut completion_capacity = CompletionCapacity::default();
        for snapshot in &unmatched_attempt_starts {
            completion_capacity.insert(snapshot);
        }
        let mut deferred_writes = VecDeque::new();
        let mut deferred_write_bytes = 0_usize;
        for terminal in recovery_writes {
            let key = (
                terminal.record.invoke_id.clone(),
                terminal.record.occurred_at.clone(),
                terminal.raw_capture,
            );
            if pending_by_key.contains_key(&key) {
                continue;
            }
            let terminal_bytes = terminal.estimated_memory_bytes();
            if deferred_writes.len() >= TERMINAL_JOURNAL_MAX_DEFERRED_WRITES
                || deferred_write_bytes.saturating_add(terminal_bytes)
                    > TERMINAL_JOURNAL_MAX_DEFERRED_BYTES
            {
                continue;
            }
            pending_by_key.insert(key, Vec::new());
            deferred_write_bytes = deferred_write_bytes.saturating_add(terminal_bytes);
            deferred_writes.push_back(terminal);
        }
        let has_pending_records = !pending_attempt_starts.is_empty()
            || !pending_by_key.is_empty()
            || !shutdown_recovery_pending.is_empty();
        let oldest_pending_at = pending_by_key
            .keys()
            .map(|(_, occurred_at, _)| occurred_at.as_str())
            .chain(
                shutdown_recovery_pending
                    .keys()
                    .map(|(_, occurred_at, _)| occurred_at.as_str()),
            )
            .chain(
                unmatched_attempt_starts
                    .iter()
                    .map(|snapshot| snapshot.occurred_at.as_str()),
            )
            .chain(
                deferred_writes
                    .iter()
                    .map(|write| write.record.occurred_at.as_str()),
            )
            .filter_map(crate::stats::parse_to_utc_datetime)
            .min();
        let pending_since = if has_pending_records {
            let now = chrono::Utc::now();
            oldest_pending_at
                .map(|occurred_at| {
                    let age_ms = now
                        .signed_duration_since(occurred_at)
                        .num_milliseconds()
                        .max(0) as u64;
                    Instant::now()
                        .checked_sub(Duration::from_millis(age_ms))
                        .unwrap_or_else(Instant::now)
                })
                .or_else(|| Some(Instant::now()))
        } else {
            None
        };
        let system_task_quarantine_ids = load_system_task_quarantine_ids(&[
            directory.join("system-task-quarantine.jsonl"),
            directory.join("shutdown-system-task-quarantine.jsonl"),
        ]);
        let deferred_system_task_finishes = load_system_task_finishes(&[
            directory.join("system-task-quarantine.jsonl"),
            directory.join("shutdown-system-task-quarantine.jsonl"),
        ]);
        let replayed_system_task_ids = deferred_system_task_finishes
            .iter()
            .map(|finish| finish.run_id)
            .collect();
        let mut journal = Self {
            directory,
            current_file: open_append_file(&current_path)?,
            segments,
            pending_by_key,
            dispatch_offsets: HashMap::new(),
            next_sequence,
            synced_sequence: next_sequence.saturating_sub(1),
            pending_bytes,
            replay_segments,
            replay_count: replay_count.saturating_add(deferred_writes.len()),
            deferred_writes,
            deferred_write_bytes,
            deferred_rejected: 0,
            recovery_dispatch_started: false,
            replay_blocked: false,
            pending_attempt_starts,
            completion_capacity,
            shutdown_recovery_pending,
            shutdown_recovery_order,
            shutdown_recovery_next_order: shutdown_recovery_ordered_keys.len() as u64,
            shutdown_recovery_source_clean,
            shutdown_recovery_bytes,
            pending_since,
            system_task_quarantine_ids,
            replayed_system_task_ids,
            system_task_ack_retry_ids: HashSet::new(),
            deferred_system_task_finishes,
            last_sync_at: Instant::now(),
            last_checkpoint_at: Instant::now(),
            overflowed: pending_bytes >= TERMINAL_JOURNAL_MAX_BYTES,
            sync_failed: false,
            append_failed: false,
        };
        let mut interrupted_by_invocation =
            HashMap::<(String, String, bool), Vec<RuntimeAttemptSnapshot>>::new();
        for snapshot in unmatched_attempt_starts {
            interrupted_by_invocation
                .entry((
                    snapshot.invoke_id.clone(),
                    snapshot.occurred_at.clone(),
                    false,
                ))
                .or_default()
                .push(snapshot);
        }
        let mut recovered_any = false;
        for ((invoke_id, occurred_at, raw_capture), mut attempts) in interrupted_by_invocation {
            attempts.sort_by_key(|attempt| attempt.attempt_index);
            let finished_at = chrono::Utc::now().to_rfc3339();
            for attempt in &mut attempts {
                attempt.finished_at = Some(finished_at.clone());
                attempt.status =
                    crate::POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_INTERRUPTED.to_string();
                attempt.phase = Some(crate::POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED.to_string());
                attempt.failure_kind =
                    Some(crate::PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED.to_string());
                attempt.error_message = Some(crate::POOL_ATTEMPT_INTERRUPTED_MESSAGE.to_string());
            }
            let record = interrupted_terminal_record(&invoke_id, &occurred_at, &attempts);
            let outcome = journal.append_with_attempts(&record, &attempts, raw_capture, None);
            if matches!(
                outcome.durability_mode,
                TerminalJournalDurabilityMode::Journal
            ) {
                let startup_backfill_tasks = startup_backfill_tasks_for_terminal(
                    &api_invocation_from_runtime_record(&record),
                );
                let recovery_write = BatchedTerminalInvocationWrite {
                    enqueued_at: None,
                    diagnostic: None,
                    record,
                    attempts,
                    capture_started: None,
                    raw_capture,
                    dashboard_terminal_sequence: Some(0),
                    terminal_projection_event_ids: Vec::new(),
                    startup_backfill_tasks,
                };
                if journal.defer_write(recovery_write.clone()) {
                    journal.replay_count = journal.replay_count.saturating_add(1);
                    recovered_any = true;
                } else {
                    journal.pending_by_key.remove(&(
                        recovery_write.record.invoke_id.clone(),
                        recovery_write.record.occurred_at.clone(),
                        recovery_write.raw_capture,
                    ));
                    journal.remember_shutdown_recovery(&[&recovery_write]);
                }
            } else {
                anyhow::bail!(
                    "cannot durably recover an accepted attempt start; journal preserved"
                );
            }
        }
        if recovered_any {
            let _ = journal.force_sync();
        }
        Ok(journal)
    }

    pub(crate) fn append(
        &mut self,
        record: &ProxyCaptureRecord,
        raw_capture: bool,
        capture_started: Option<Instant>,
    ) -> TerminalJournalAppendOutcome {
        self.append_with_attempts(record, &[], raw_capture, capture_started)
    }

    pub(crate) fn append_with_attempts(
        &mut self,
        record: &ProxyCaptureRecord,
        attempts: &[RuntimeAttemptSnapshot],
        raw_capture: bool,
        capture_started: Option<Instant>,
    ) -> TerminalJournalAppendOutcome {
        let key = (
            record.invoke_id.clone(),
            record.occurred_at.clone(),
            raw_capture,
        );
        if self.replay_blocked || self.sync_failed || self.append_failed {
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
            attempts: attempts.iter().map(journal_safe_attempt_snapshot).collect(),
            record: journal_safe_terminal_record(record),
        };
        let Ok(encoded) = encode_entry_line(&entry) else {
            warn!(invoke_id = %record.invoke_id, occurred_at = %record.occurred_at, "terminal journal serialization failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        };
        let encoded_len = encoded.len() as u64;
        if encoded.len() > TERMINAL_JOURNAL_REPLAY_MAX_BYTES {
            self.overflowed = true;
            warn!(
                invoke_id = %record.invoke_id,
                occurred_at = %record.occurred_at,
                encoded_bytes = encoded.len(),
                max_bytes = TERMINAL_JOURNAL_REPLAY_MAX_BYTES,
                "terminal journal event exceeds the bounded replay size"
            );
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        let mut acknowledged_ids = HashSet::new();
        let acknowledgement_sequences = attempts
            .iter()
            .filter(|attempt| acknowledged_ids.insert(attempt.attempt_public_id.as_str()))
            .filter_map(|attempt| {
                self.pending_attempt_starts
                    .get(&attempt.attempt_public_id)
                    .copied()
                    .map(|sequence| (attempt.attempt_public_id.as_str(), sequence))
            })
            .collect::<Vec<_>>();
        let acknowledgement_bytes = acknowledgement_sequences
            .iter()
            .filter_map(|(_, sequence)| encode_acknowledgement_line(*sequence).ok())
            .map(|line| line.len() as u64)
            .sum::<u64>();
        let acknowledged_ids = acknowledgement_sequences
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let remaining_reservation = self
            .completion_capacity
            .reserved_bytes()
            .saturating_sub(self.completion_capacity.released_bytes(&acknowledged_ids));
        if self
            .pending_bytes
            .saturating_add(remaining_reservation)
            .saturating_add(encoded_len)
            .saturating_add(acknowledgement_bytes)
            > TERMINAL_JOURNAL_MAX_BYTES
        {
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
            self.append_failed = true;
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
        self.dispatch_offsets
            .insert(key.clone(), (current_start, segment.bytes, entry.sequence));
        segment.last_sequence = entry.sequence;
        segment.bytes = segment.bytes.saturating_add(encoded_len);
        segment.sequences.insert(entry.sequence);
        self.pending_by_key.insert(key, vec![entry.sequence]);
        if self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.pending_bytes = self.pending_bytes.saturating_add(encoded_len);
        let mut acknowledgement_failed = false;
        for (attempt_public_id, start_sequence) in acknowledgement_sequences {
            let Ok(acknowledgement) = encode_acknowledgement_line(start_sequence) else {
                warn!(
                    sequence = start_sequence,
                    "terminal journal attempt acknowledgement serialization failed"
                );
                self.append_failed = true;
                self.overflowed = true;
                acknowledgement_failed = true;
                continue;
            };
            if let Err(error) = self.current_file.write_all(&acknowledgement) {
                warn!(error = %error, sequence = start_sequence, "terminal journal attempt acknowledgement append failed");
                self.append_failed = true;
                self.overflowed = true;
                acknowledgement_failed = true;
                continue;
            }
            let acknowledgement_len = acknowledgement.len() as u64;
            let current_start = self
                .segments
                .iter()
                .find_map(|(start, segment)| segment.current.then_some(*start))
                .expect("current terminal journal segment exists");
            let segment = self
                .segments
                .get_mut(&current_start)
                .expect("current terminal journal segment exists");
            segment.bytes = segment.bytes.saturating_add(acknowledgement_len);
            if let Some(start_segment) = self
                .segments
                .values_mut()
                .find(|segment| segment.sequences.contains(&start_sequence))
            {
                start_segment.acknowledged.insert(start_sequence);
            }
            self.pending_bytes = self.pending_bytes.saturating_add(acknowledgement_len);
            self.pending_attempt_starts.remove(attempt_public_id);
            self.completion_capacity.release(attempt_public_id);
        }
        if acknowledgement_failed {
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        self.append_outcome(TerminalJournalDurabilityMode::Journal, Some(entry.sequence))
    }

    pub(crate) fn append_attempt_start(
        &mut self,
        snapshot: &RuntimeAttemptSnapshot,
    ) -> TerminalJournalAppendOutcome {
        if self.sync_failed
            || self.append_failed
            || self
                .pending_attempt_starts
                .contains_key(&snapshot.attempt_public_id)
        {
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        let event = JournalAttemptStart {
            event_version: 1,
            sequence: self.next_sequence,
            attempt: journal_safe_attempt_snapshot(snapshot),
        };
        let Ok(encoded) = encode_attempt_start_line(&event) else {
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        };
        let encoded_len = encoded.len() as u64;
        if encoded.len() > TERMINAL_JOURNAL_REPLAY_MAX_BYTES {
            self.overflowed = true;
            warn!(
                attempt_public_id = %snapshot.attempt_public_id,
                encoded_bytes = encoded.len(),
                max_bytes = TERMINAL_JOURNAL_REPLAY_MAX_BYTES,
                "terminal journal attempt-start event exceeds the bounded replay size"
            );
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        let completion_increment = self.completion_capacity.start_increment(snapshot);
        if self
            .pending_bytes
            .saturating_add(encoded_len)
            .saturating_add(self.completion_capacity.reserved_bytes())
            .saturating_add(completion_increment)
            > TERMINAL_JOURNAL_MAX_BYTES
        {
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if let Err(err) = self.rotate_if_needed(encoded_len) {
            warn!(error = %err, "terminal journal start-event rotation failed");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if let Err(err) = self.current_file.write_all(&encoded) {
            warn!(error = %err, "terminal journal start-event append failed");
            self.overflowed = true;
            self.append_failed = true;
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
        segment.last_sequence = event.sequence;
        segment.bytes = segment.bytes.saturating_add(encoded_len);
        segment.sequences.insert(event.sequence);
        self.pending_bytes = self.pending_bytes.saturating_add(encoded_len);
        self.completion_capacity.insert(snapshot);
        self.pending_attempt_starts
            .insert(snapshot.attempt_public_id.clone(), event.sequence);
        if self.pending_since.is_none() {
            self.pending_since = Some(Instant::now());
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.append_outcome(TerminalJournalDurabilityMode::Journal, Some(event.sequence))
    }

    pub(crate) fn acknowledge_attempt_start(&mut self, attempt_public_id: &str) -> bool {
        let Some(sequence) = self.pending_attempt_starts.get(attempt_public_id).copied() else {
            return false;
        };
        let Ok(encoded) = encode_acknowledgement_line(sequence) else {
            warn!(
                sequence,
                "terminal journal attempt acknowledgement serialization failed"
            );
            self.append_failed = true;
            self.overflowed = true;
            return false;
        };
        if self.pending_bytes.saturating_add(encoded.len() as u64) > TERMINAL_JOURNAL_MAX_BYTES {
            self.overflowed = true;
            return false;
        }
        if let Err(error) = self.current_file.write_all(&encoded) {
            warn!(error = %error, attempt_public_id, "terminal journal attempt acknowledgement append failed");
            self.append_failed = true;
            self.overflowed = true;
            return false;
        }
        let current_start = self
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current terminal journal segment exists");
        {
            let segment = self
                .segments
                .get_mut(&current_start)
                .expect("current terminal journal segment exists");
            segment.bytes = segment.bytes.saturating_add(encoded.len() as u64);
        }
        let acknowledgement_target = self
            .segments
            .iter()
            .find_map(|(start, segment)| segment.sequences.contains(&sequence).then_some(*start));
        if let Some(target) = acknowledgement_target {
            self.segments
                .get_mut(&target)
                .expect("terminal journal acknowledgement target exists")
                .acknowledged
                .insert(sequence);
        }
        self.pending_bytes = self.pending_bytes.saturating_add(encoded.len() as u64);
        self.pending_attempt_starts.remove(attempt_public_id);
        self.completion_capacity.release(attempt_public_id);
        true
    }

    pub(crate) fn reserve_attempt_completion_growth(
        &mut self,
        snapshot: &RuntimeAttemptSnapshot,
    ) -> bool {
        if self.sync_failed || self.append_failed {
            return false;
        }
        let accepted = self
            .completion_capacity
            .reserve_growth(snapshot, self.pending_bytes);
        if !accepted {
            self.overflowed = true;
        }
        accepted
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
                self.synced_sequence = self.next_sequence.saturating_sub(1);
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
                self.synced_sequence = self.next_sequence.saturating_sub(1);
                Ok(())
            }
            Err(err) => {
                self.overflowed = true;
                self.sync_failed = true;
                Err(err).context("failed to sync terminal journal")
            }
        }
    }

    pub(crate) fn pending_identity_keys(&self) -> Vec<(String, String, bool)> {
        self.pending_by_key
            .keys()
            .chain(self.shutdown_recovery_pending.keys())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    pub(crate) fn sequence_is_synced(&self, sequence: u64) -> Result<bool> {
        if self.replay_blocked || self.sync_failed || self.append_failed {
            return Err(anyhow::anyhow!("runtime journal group sync failed"));
        }
        Ok(sequence <= self.synced_sequence)
    }

    #[cfg(test)]
    pub(crate) fn fail_append_for_test(&mut self) {
        self.append_failed = true;
    }

    pub(crate) fn defer_from_log(&mut self, id: &str, occurred_at: &str, raw: bool) -> bool {
        let key = (id.to_string(), occurred_at.to_string(), raw);
        let Some((first_sequence, byte_offset, sequence)) = self.dispatch_offsets.remove(&key)
        else {
            return false;
        };
        self.replay_segments.push_back(JournalReplayCursor {
            first_sequence,
            byte_offset,
            limit_sequence: Some(sequence),
        });
        true
    }

    pub(crate) fn identity_pending(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        raw_capture: bool,
    ) -> bool {
        let key = (invoke_id.to_owned(), occurred_at.to_owned(), raw_capture);
        self.pending_by_key.contains_key(&key) || self.shutdown_recovery_pending.contains_key(&key)
    }

    pub(crate) fn acknowledge(&mut self, invoke_id: &str, occurred_at: &str, raw_capture: bool) {
        let key = (invoke_id.to_string(), occurred_at.to_string(), raw_capture);
        self.dispatch_offsets.remove(&key);
        let recovery_pending = self.shutdown_recovery_pending.contains_key(&key);
        let Some(sequences) = self
            .pending_by_key
            .get(&key)
            .cloned()
            .or_else(|| recovery_pending.then_some(Vec::new()))
        else {
            return;
        };
        let recovery_only = recovery_pending && sequences.is_empty();
        let mut recovery_ack_durable = true;
        if recovery_pending {
            if let Err(err) = append_shutdown_terminal_ack(
                &self.directory.join("shutdown-terminal-quarantine.jsonl"),
                invoke_id,
                occurred_at,
                raw_capture,
            ) {
                warn!(error = %err, invoke_id, occurred_at, "failed to checkpoint shutdown terminal recovery acknowledgement");
                self.shutdown_recovery_source_clean = false;
                recovery_ack_durable = false;
                if recovery_only {
                    return;
                }
            } else if let Some((record, attempts)) = self.shutdown_recovery_pending.remove(&key) {
                self.shutdown_recovery_bytes = self.shutdown_recovery_bytes.saturating_sub(
                    record.estimated_memory_bytes()
                        + attempts
                            .iter()
                            .map(RuntimeAttemptSnapshot::estimated_memory_bytes)
                            .sum::<usize>(),
                );
            }
        }
        let mut pending_sequences = Vec::new();
        let mut written_sequences = Vec::new();
        for sequence in sequences {
            let Ok(encoded) = encode_acknowledgement_line(sequence) else {
                warn!(
                    sequence,
                    "terminal journal acknowledgement serialization failed"
                );
                self.append_failed = true;
                self.overflowed = true;
                pending_sequences.push(sequence);
                continue;
            };
            if let Err(err) = self.current_file.write_all(&encoded) {
                warn!(error = %err, sequence, "terminal journal acknowledgement append failed");
                self.append_failed = true;
                self.overflowed = true;
                pending_sequences.push(sequence);
                continue;
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
            written_sequences.push(sequence);
        }
        let mut acknowledged_replay_count = 0_usize;
        if !written_sequences.is_empty() {
            if let Err(err) = self.current_file.sync_data() {
                warn!(
                    error = %err,
                    count = written_sequences.len(),
                    "terminal journal acknowledgement checkpoint failed"
                );
                self.sync_failed = true;
                self.overflowed = true;
                pending_sequences.extend(written_sequences);
            } else {
                self.last_sync_at = Instant::now();
                for sequence in written_sequences {
                    let acknowledgement_target =
                        self.segments.iter().find_map(|(start, segment)| {
                            segment.sequences.contains(&sequence).then_some(*start)
                        });
                    if let Some(start) = acknowledgement_target {
                        self.segments
                            .get_mut(&start)
                            .expect("terminal journal acknowledgement target exists")
                            .acknowledged
                            .insert(sequence);
                    }
                    acknowledged_replay_count = acknowledged_replay_count.saturating_add(1);
                }
            }
        }
        if pending_sequences.is_empty() {
            self.pending_by_key.remove(&key);
        } else {
            self.pending_by_key.insert(key, pending_sequences);
        }
        self.last_checkpoint_at = Instant::now();
        self.replay_count = self
            .replay_count
            .saturating_sub(acknowledged_replay_count + usize::from(recovery_only));
        self.remove_fully_acknowledged_segments();
        if self.pending_by_key.is_empty()
            && self.pending_attempt_starts.is_empty()
            && self.shutdown_recovery_pending.is_empty()
        {
            self.pending_since = None;
        }
        if recovery_pending
            && recovery_ack_durable
            && self.shutdown_recovery_source_clean
            && let Err(err) = compact_shutdown_terminal_recovery(
                &self.directory.join("shutdown-terminal-quarantine.jsonl"),
                &self.shutdown_recovery_entries_in_order(),
            )
        {
            warn!(error = %err, "failed to compact shutdown terminal recovery sink");
        }
        if !self.sync_failed
            && !self.append_failed
            && self
                .pending_bytes
                .saturating_add(self.completion_capacity.reserved_bytes())
                < TERMINAL_JOURNAL_MAX_BYTES * 3 / 5
        {
            self.overflowed = false;
        }
    }

    fn shutdown_recovery_entries_in_order(
        &self,
    ) -> Vec<(ShutdownRecoveryKey, ShutdownRecoveryValue)> {
        let mut entries = self
            .shutdown_recovery_pending
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(key, _)| {
            self.shutdown_recovery_order
                .get(key)
                .copied()
                .unwrap_or(u64::MAX)
        });
        entries
    }

    pub(crate) fn remember_shutdown_recovery(
        &mut self,
        terminals: &[&BatchedTerminalInvocationWrite],
    ) -> bool {
        let mut accepted = true;
        for terminal in terminals {
            let key = (
                terminal.record.invoke_id.clone(),
                terminal.record.occurred_at.clone(),
                terminal.raw_capture,
            );
            let value = (
                journal_safe_terminal_record(&terminal.record),
                terminal
                    .attempts
                    .iter()
                    .map(journal_safe_attempt_snapshot)
                    .collect::<Vec<_>>(),
            );
            let value_bytes = value.0.estimated_memory_bytes()
                + value
                    .1
                    .iter()
                    .map(RuntimeAttemptSnapshot::estimated_memory_bytes)
                    .sum::<usize>();
            let existing_bytes = self
                .shutdown_recovery_pending
                .get(&key)
                .map(|(record, attempts)| {
                    record.estimated_memory_bytes()
                        + attempts
                            .iter()
                            .map(RuntimeAttemptSnapshot::estimated_memory_bytes)
                            .sum::<usize>()
                })
                .unwrap_or_default();
            if !self.shutdown_recovery_pending.contains_key(&key)
                && (self.shutdown_recovery_pending.len() >= TERMINAL_JOURNAL_MAX_DEFERRED_WRITES
                    || self
                        .shutdown_recovery_bytes
                        .saturating_sub(existing_bytes)
                        .saturating_add(value_bytes)
                        > TERMINAL_JOURNAL_MAX_DEFERRED_BYTES)
            {
                self.deferred_rejected = self.deferred_rejected.saturating_add(1);
                self.overflowed = true;
                // The durable recovery file now contains a fact that is absent from the
                // bounded in-memory projection. Do not let a later ACK compact it away.
                self.shutdown_recovery_source_clean = false;
                accepted = false;
                continue;
            }
            if self
                .shutdown_recovery_pending
                .insert(key.clone(), value)
                .map(|previous| {
                    self.shutdown_recovery_bytes = self.shutdown_recovery_bytes.saturating_sub(
                        previous.0.estimated_memory_bytes()
                            + previous
                                .1
                                .iter()
                                .map(RuntimeAttemptSnapshot::estimated_memory_bytes)
                                .sum::<usize>(),
                    );
                })
                .is_none()
                && !self.pending_by_key.contains_key(&key)
            {
                self.replay_count = self.replay_count.saturating_add(1);
                self.shutdown_recovery_order
                    .insert(key.clone(), self.shutdown_recovery_next_order);
                self.shutdown_recovery_next_order =
                    self.shutdown_recovery_next_order.saturating_add(1);
            }
            self.shutdown_recovery_bytes = self.shutdown_recovery_bytes.saturating_add(value_bytes);
            if self.pending_since.is_none() {
                self.pending_since = Some(Instant::now());
            }
        }
        accepted
    }

    pub(crate) fn take_replay(&mut self) -> Vec<BatchedTerminalInvocationWrite> {
        self.take_replay_chunk(usize::MAX, usize::MAX)
    }

    pub(crate) fn start_recovery_dispatch(&mut self, max_writes: usize) -> usize {
        self.recovery_dispatch_started = true;
        self.queue_replay_for_dispatch(max_writes);
        self.replay_count
    }

    pub(crate) fn start_recovery_dispatch_p1_reservation(&mut self, max_writes: usize) -> usize {
        self.start_recovery_dispatch(max_writes);
        terminal_journal_p1_reservation(self.stats())
    }

    pub(crate) fn queue_replay_for_dispatch(&mut self, max_writes: usize) {
        if max_writes == 0 || self.deferred_writes.len() >= max_writes {
            return;
        }
        let available = max_writes.saturating_sub(self.deferred_writes.len());
        let replay = self.take_replay_chunk(available, TERMINAL_JOURNAL_REPLAY_MAX_BYTES);
        for write in replay {
            if !self.defer_write(write.clone()) {
                let key = (
                    write.record.invoke_id.clone(),
                    write.record.occurred_at.clone(),
                    write.raw_capture,
                );
                self.pending_by_key.remove(&key);
                self.overflowed = true;
                let recovery_persisted = self
                    .quarantine_shutdown_batch(
                        &[&write],
                        &[],
                        "terminal journal replay dispatch capacity",
                    )
                    .is_ok();
                if !recovery_persisted {
                    self.replay_blocked = true;
                    warn!(
                        invoke_id = %write.record.invoke_id,
                        occurred_at = %write.record.occurred_at,
                        "terminal journal replay could not persist a shutdown recovery obligation"
                    );
                } else if !self.remember_shutdown_recovery(&[&write]) {
                    self.replay_blocked = true;
                    warn!(
                        invoke_id = %write.record.invoke_id,
                        occurred_at = %write.record.occurred_at,
                        "terminal journal replay could not preserve a deferred write in shutdown recovery"
                    );
                } else {
                    // Moving the record to the durable shutdown-recovery projection transfers
                    // the existing replay obligation; it must not create a second count.
                    self.replay_count = self.replay_count.saturating_sub(1);
                }
            }
        }
        let available = max_writes.saturating_sub(self.deferred_writes.len());
        self.queue_shutdown_recovery_for_dispatch(available);
    }

    pub(crate) fn recovery_dispatch_started(&self) -> bool {
        self.recovery_dispatch_started
    }

    pub(crate) fn system_task_recovery_pending(&self) -> bool {
        !self.deferred_system_task_finishes.is_empty() || !self.replayed_system_task_ids.is_empty()
    }

    pub(crate) fn forget_system_task_recovery_for_runs(&mut self, run_ids: &[i64]) {
        let run_ids = run_ids.iter().copied().collect::<HashSet<_>>();
        self.replayed_system_task_ids
            .retain(|run_id| !run_ids.contains(run_id));
        self.system_task_ack_retry_ids
            .retain(|run_id| !run_ids.contains(run_id));
    }

    pub(crate) fn defer_system_task_acknowledgements(&mut self, run_ids: &[i64]) {
        self.system_task_ack_retry_ids.extend(
            run_ids
                .iter()
                .copied()
                .filter(|run_id| self.replayed_system_task_ids.contains(run_id)),
        );
    }

    pub(crate) fn retry_system_task_acknowledgements(&mut self) -> Result<usize> {
        let run_ids = self
            .system_task_ack_retry_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        if run_ids.is_empty() {
            return Ok(0);
        }
        self.acknowledge_system_task_finishes(&run_ids)
            .map(|()| run_ids.len())
    }

    fn queue_shutdown_recovery_for_dispatch(&mut self, max_writes: usize) {
        if max_writes == 0 {
            return;
        }
        let keys = self
            .shutdown_recovery_pending
            .keys()
            .filter(|key| !self.pending_by_key.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        let mut keys = keys;
        keys.sort_by_key(|key| {
            self.shutdown_recovery_order
                .get(key)
                .copied()
                .unwrap_or(u64::MAX)
        });
        let keys = keys.into_iter().take(max_writes).collect::<Vec<_>>();
        for key in keys {
            let Some((record, attempts)) = self.shutdown_recovery_pending.get(&key).cloned() else {
                continue;
            };
            let write = BatchedTerminalInvocationWrite {
                enqueued_at: None,
                diagnostic: None,
                capture_started: None,
                raw_capture: key.2,
                attempts,
                dashboard_terminal_sequence: Some(0),
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: startup_backfill_tasks_for_terminal(
                    &api_invocation_from_runtime_record(&record),
                ),
                record,
            };
            if self.defer_write(write) {
                self.pending_by_key.insert(key, Vec::new());
            } else {
                break;
            }
        }
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
                self.replay_blocked = true;
                warn!(path = %segment.path.display(), "terminal journal replay segment could not be opened");
                break;
            };
            let mut reader = BufReader::new(file);
            if reader.seek(SeekFrom::Start(cursor.byte_offset)).is_err() {
                self.replay_blocked = true;
                warn!(path = %segment.path.display(), "terminal journal replay segment could not be seeked");
                break;
            }
            let mut reached_end = false;
            let mut produced = None;
            loop {
                let stream_start = cursor.byte_offset;
                let mut raw_line = Vec::new();
                let read_bytes = {
                    let mut budget_reader = ReplayBudgetReader {
                        reader: &mut reader,
                        remaining: TERMINAL_JOURNAL_REPLAY_MAX_BYTES,
                    };
                    budget_reader.read_until(b'\n', &mut raw_line)
                };
                let stream_end = match reader.stream_position() {
                    Ok(stream_end) => stream_end,
                    Err(error) => {
                        self.replay_blocked = true;
                        warn!(error = %error, path = %segment.path.display(), "terminal journal replay segment position could not be read");
                        break;
                    }
                };
                let Ok(read_bytes) = read_bytes else {
                    cursor.byte_offset = stream_start;
                    self.replay_blocked = true;
                    warn!(
                        replay_offset = stream_start,
                        "terminal journal replay exceeded the bounded record input budget; preserving the cursor for recovery"
                    );
                    break;
                };
                if read_bytes == 0 || stream_end == stream_start {
                    reached_end = true;
                    break;
                }
                if raw_line.last().copied() != Some(b'\n') {
                    cursor.byte_offset = stream_start;
                    self.replay_blocked = true;
                    warn!(
                        replay_offset = stream_start,
                        "terminal journal replay stopped at an incomplete entry; preserving the cursor for recovery"
                    );
                    break;
                }
                scanned_bytes = scanned_bytes.saturating_add(
                    stream_end
                        .saturating_sub(stream_start)
                        .min(usize::MAX as u64) as usize,
                );
                scanned_lines = scanned_lines.saturating_add(1);
                if raw_line.iter().all(u8::is_ascii_whitespace) {
                    cursor.byte_offset = stream_end;
                    if scanned_bytes >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES
                        || scanned_lines >= TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES
                    {
                        break;
                    }
                    continue;
                }
                let parsed = match segment_recovery::verified_frame(&raw_line) {
                    Ok(parsed) => parsed,
                    Err(reason) => {
                        cursor.byte_offset = stream_start;
                        self.replay_blocked = true;
                        warn!(
                            replay_offset = stream_start,
                            reason,
                            "terminal journal replay stopped at an invalid entry; preserving the cursor for recovery"
                        );
                        break;
                    }
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
                if cursor
                    .limit_sequence
                    .is_some_and(|limit| entry.sequence > limit)
                {
                    reached_end = true;
                    break;
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
                    enqueued_at: None,
                    diagnostic: None,
                    capture_started: entry.capture_elapsed_ms.and_then(|elapsed_ms| {
                        Instant::now().checked_sub(Duration::from_millis(elapsed_ms))
                    }),
                    raw_capture: entry.raw_capture,
                    attempts: entry.attempts,
                    // The original process-local dashboard sequence is unavailable after a
                    // restart. The writer treats this sentinel as an exact replay overlay.
                    dashboard_terminal_sequence: Some(0),
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
                if cursor.limit_sequence.is_some() {
                    self.replay_segments.pop_front();
                }
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
        let bytes = write.estimated_memory_bytes();
        if self.deferred_writes.len() >= TERMINAL_JOURNAL_MAX_DEFERRED_WRITES
            || self.deferred_write_bytes.saturating_add(bytes) > TERMINAL_JOURNAL_MAX_DEFERRED_BYTES
        {
            self.deferred_rejected = self.deferred_rejected.saturating_add(1);
            return false;
        }
        self.deferred_write_bytes = self.deferred_write_bytes.saturating_add(bytes);
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
        let writes = self.deferred_writes.drain(..count).collect::<Vec<_>>();
        self.deferred_write_bytes = self
            .deferred_writes
            .iter()
            .map(BatchedTerminalInvocationWrite::estimated_memory_bytes)
            .sum();
        writes
    }

    pub(crate) fn stats(&self) -> TerminalJournalStats {
        TerminalJournalStats {
            pending_records: self
                .pending_by_key
                .len()
                .saturating_add(self.pending_attempt_starts.len())
                .saturating_add(self.shutdown_recovery_pending.len()),
            pending_materialization_records: self
                .pending_by_key
                .len()
                .saturating_add(self.shutdown_recovery_pending.len()),
            pending_bytes: self.pending_bytes,
            completion_reserved_bytes: self.completion_capacity.reserved_bytes(),
            segment_count: self.segments.len(),
            replay_count: self.replay_count,
            checkpoint_lag_ms: self.last_checkpoint_at.elapsed().as_millis() as u64,
            oldest_pending_age_ms: self
                .pending_since
                .map_or(0, |instant| instant.elapsed().as_millis() as u64),
            overflowed: self.overflowed,
            unavailable: self.replay_blocked || self.sync_failed || self.append_failed,
            deferred_records: self.deferred_writes.len(),
            deferred_bytes: self.deferred_write_bytes,
            deferred_rejected: self.deferred_rejected,
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
        self.synced_sequence = self.next_sequence.saturating_sub(1);
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

impl std::io::BufRead for ReplayBudgetReader<'_> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let available = self.reader.fill_buf()?;
        if self.remaining == 0 && !available.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal journal replay record exceeds the bounded input budget",
            ));
        }
        Ok(&available[..available.len().min(self.remaining)])
    }

    fn consume(&mut self, amount: usize) {
        let bounded = amount.min(self.remaining);
        self.reader.consume(bounded);
        self.remaining = self.remaining.saturating_sub(bounded);
    }
}

fn append_shutdown_terminal_ack(
    path: &Path,
    invoke_id: &str,
    occurred_at: &str,
    raw_capture: bool,
) -> Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "shutdown terminal recovery file is missing: {}",
            path.display()
        ));
    }
    let mut file = open_shutdown_recovery_append(path, false).with_context(|| {
        format!(
            "failed to open shutdown terminal recovery {}",
            path.display()
        )
    })?;
    serde_json::to_writer(
        &mut file,
        &ShutdownTerminalRecoveryLine {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            raw_capture,
            acknowledged: true,
            error: None,
            record: None,
            attempts: Vec::new(),
        },
    )
    .context("failed to encode shutdown terminal recovery acknowledgement")?;
    file.write_all(b"\n")
        .context("failed to append shutdown terminal recovery acknowledgement")?;
    file.sync_data()
        .context("failed to sync shutdown terminal recovery acknowledgement")?;
    Ok(())
}

fn load_system_task_quarantine_ids(paths: &[PathBuf]) -> HashSet<i64> {
    let mut ids = HashSet::new();
    for path in paths {
        let Ok(file) = File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(read) = reader.read_line(&mut line) else {
                break;
            };
            if read == 0 {
                break;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                && let Some(run_id) = value.get("run_id").and_then(serde_json::Value::as_i64)
            {
                ids.insert(run_id);
            }
        }
    }
    ids
}

fn append_system_task_acknowledgements(path: &Path, run_ids: &[i64]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open system-task quarantine {}", path.display()))?;
    for run_id in run_ids {
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({ "run_id": run_id, "acknowledged": true }),
        )
        .context("failed to encode system-task quarantine acknowledgement")?;
        file.write_all(b"\n")
            .context("failed to append system-task quarantine acknowledgement")?;
    }
    file.sync_data()
        .context("failed to sync system-task quarantine acknowledgement")?;
    Ok(())
}

fn load_system_task_finishes(paths: &[PathBuf]) -> VecDeque<BatchedSystemTaskFinish> {
    let mut entries = HashMap::<i64, (Option<BatchedSystemTaskFinish>, bool)>::new();
    for path in paths {
        let Ok(file) = File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(read) = reader.read_line(&mut line) else {
                break;
            };
            if read == 0 {
                break;
            }
            let Ok(value) = serde_json::from_str::<SystemTaskRecoveryLine>(&line) else {
                continue;
            };
            if value.acknowledged {
                entries.insert(value.run_id, (None, true));
                continue;
            }
            let (Some(task_kind), Some(status)) = (
                parse_system_task_kind(&value.task_kind),
                parse_system_task_status(&value.status),
            ) else {
                continue;
            };
            if entries
                .get(&value.run_id)
                .is_some_and(|(_, acknowledged)| *acknowledged)
            {
                continue;
            }
            entries.insert(
                value.run_id,
                (
                    Some(BatchedSystemTaskFinish {
                        run_id: value.run_id,
                        task_key: if value.task_key.is_empty() {
                            task_kind.as_str().to_string()
                        } else {
                            value.task_key
                        },
                        task_kind,
                        trigger_kind: value.trigger_kind,
                        status,
                        summary: value.summary,
                        detail: value.detail,
                        finished_at: value.finished_at,
                        duration_ms: value.duration_ms,
                        completion: value.completion,
                        core_completion: value.core_completion,
                        details: value.details,
                    }),
                    false,
                ),
            );
        }
    }
    entries
        .into_values()
        .filter_map(|(finish, acknowledged)| (!acknowledged).then_some(finish).flatten())
        .collect()
}

fn parse_system_task_kind(value: &str) -> Option<SystemTaskKind> {
    match value {
        "retention_archive" => Some(SystemTaskKind::RetentionArchive),
        "prompt_cache_materialization" => Some(SystemTaskKind::PromptCacheMaterialization),
        "startup_backfill" => Some(SystemTaskKind::StartupBackfill),
        "hourly_rollup_bootstrap" | "startup_hourly_rollup_bootstrap" => {
            Some(SystemTaskKind::HourlyRollupBootstrap)
        }
        "forward_proxy_subscription_refresh" => {
            Some(SystemTaskKind::ForwardProxySubscriptionRefresh)
        }
        _ => None,
    }
}

fn parse_system_task_status(value: &str) -> Option<SystemTaskStatus> {
    match value {
        "running" => Some(SystemTaskStatus::Running),
        "success" => Some(SystemTaskStatus::Success),
        "failed" => Some(SystemTaskStatus::Failed),
        "skipped" => Some(SystemTaskStatus::Skipped),
        _ => None,
    }
}

fn segment_path(directory: &Path, first_sequence: u64) -> PathBuf {
    directory.join(format!("terminal-{first_sequence:020}.jsonl"))
}

pub(crate) fn terminal_journal_directory(database_path: &Path) -> PathBuf {
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
        event_version: TERMINAL_JOURNAL_EVENT_VERSION,
        entry: Some(entry.clone()),
        attempt_start: None,
        acknowledged_sequence: None,
        checksum: journal_line_checksum(TERMINAL_JOURNAL_EVENT_VERSION, Some(entry), None, None)?,
    };
    let mut bytes = serde_json::to_vec(&line).context("failed to encode terminal journal line")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn encode_attempt_start_line(event: &JournalAttemptStart) -> Result<Vec<u8>> {
    let line = JournalLine {
        event_version: TERMINAL_JOURNAL_EVENT_VERSION,
        entry: None,
        attempt_start: Some(event.clone()),
        acknowledged_sequence: None,
        checksum: journal_line_checksum(TERMINAL_JOURNAL_EVENT_VERSION, None, Some(event), None)?,
    };
    let mut bytes =
        serde_json::to_vec(&line).context("failed to encode terminal journal start event")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn encode_acknowledgement_line(sequence: u64) -> Result<Vec<u8>> {
    let line = JournalLine {
        event_version: TERMINAL_JOURNAL_EVENT_VERSION,
        entry: None,
        attempt_start: None,
        acknowledged_sequence: Some(sequence),
        checksum: journal_line_checksum(
            TERMINAL_JOURNAL_EVENT_VERSION,
            None,
            None,
            Some(sequence),
        )?,
    };
    let mut bytes =
        serde_json::to_vec(&line).context("failed to encode terminal journal acknowledgement")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn journal_line_checksum(
    event_version: u8,
    entry: Option<&JournalTerminalRecord>,
    attempt_start: Option<&JournalAttemptStart>,
    acknowledged_sequence: Option<u64>,
) -> Result<String> {
    let encoded = serde_json::to_vec(&(event_version, entry, attempt_start, acknowledged_sequence))
        .context("failed to checksum terminal journal event")?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

#[cfg(test)]
pub(crate) use tests::test_attempt_snapshot;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_terminal_journal_reserves_only_deferred_p1_records() {
        assert_eq!(
            terminal_journal_p1_reservation(TerminalJournalStats {
                replay_count: 4,
                deferred_records: 1,
                unavailable: true,
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            terminal_journal_p1_reservation(TerminalJournalStats {
                replay_count: 4,
                deferred_records: 1,
                ..Default::default()
            }),
            4
        );
    }

    pub(crate) fn test_attempt_snapshot() -> RuntimeAttemptSnapshot {
        RuntimeAttemptSnapshot {
            routing_control_fingerprint: None,
            attempt_public_id: "attempt-journal-1".to_string(),
            invoke_id: "journal-invoke-1".to_string(),
            occurred_at: "2026-07-29T00:00:00Z".to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-secret".to_string()),
            routing_source: Some("pool".to_string()),
            routing_selection_audit_json: Some("audit-secret".to_string()),
            requester_ip: Some("192.0.2.1".to_string()),
            upstream_base_url_host: Some("example.invalid".to_string()),
            group_name_snapshot: Some("group-a".to_string()),
            proxy_binding_key_snapshot: Some("binding-secret".to_string()),
            request_model: Some("gpt-test".to_string()),
            upstream_request_model: Some("gpt-upstream".to_string()),
            model_mapping_pattern: None,
            request_summary_json: Some("request-summary-secret".to_string()),
            upstream_account_id: 7,
            upstream_route_key: "https://journal-user:journal-password@example.invalid/v1"
                .to_string(),
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 0,
            started_at: "2026-07-29T00:00:00Z".to_string(),
            finished_at: None,
            status: "pending".to_string(),
            phase: Some("connecting".to_string()),
            http_status: None,
            downstream_http_status: None,
            failure_kind: None,
            error_message: Some("error-secret".to_string()),
            downstream_error_message: Some("downstream-secret".to_string()),
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            stream_latency_ms: None,
            upstream_request_id: Some("request-id-secret".to_string()),
            compact_support_status: None,
            compact_support_reason: None,
            upstream_request_compression_algorithm: None,
            upstream_request_compression_mode: None,
            upstream_request_logical_body_bytes: None,
            upstream_request_transmitted_body_bytes: None,
            upstream_request_header_bytes_approx: None,
            upstream_response_body_bytes: None,
            upstream_response_header_bytes_approx: None,
            response_raw_path: None,
            response_raw_codec: None,
            response_raw_size: None,
            response_raw_truncated: false,
            response_raw_truncated_reason: None,
            response_content_encoding: None,
        }
    }

    #[test]
    fn attempt_start_is_journaled_before_terminal_materialization() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-attempt-start-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut attempt = test_attempt_snapshot();
        attempt.compact_support_reason = Some("compact-error-secret".to_string());
        let record =
            crate::tests::test_proxy_capture_record(&attempt.invoke_id, &attempt.occurred_at);

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        assert_eq!(
            journal.append_attempt_start(&attempt).durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
        journal.force_sync().expect("sync start event");
        let directory = terminal_journal_directory(&database_path);
        let contents = fs::read_dir(&directory)
            .expect("read journal directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("terminal-"))
            .map(|entry| fs::read_to_string(entry.path()).expect("read journal segment"))
            .collect::<String>();
        assert!(contents.contains(&attempt.attempt_public_id));
        assert!(!contents.contains("request-summary-secret"));
        assert!(!contents.contains("error-secret"));
        assert!(!contents.contains("sticky-secret"));
        assert!(!contents.contains("journal-user"));
        assert!(!contents.contains("journal-password"));
        assert!(!contents.contains("compact-error-secret"));
        assert!(contents.contains("example.invalid"));

        journal
            .append_with_attempts(&record, &[attempt], false, None)
            .sequence
            .expect("append terminal event");
        journal.force_sync().expect("sync terminal event");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(reopened.stats().pending_records, 1);
        let replay = reopened.take_replay();
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].attempts.len(), 1);
        assert_eq!(replay[0].attempts[0].attempt_public_id, "attempt-journal-1");
        assert!(replay[0].attempts[0].request_summary_json.is_none());
        assert!(replay[0].attempts[0].requester_ip.is_none());
        assert!(replay[0].attempts[0].compact_support_reason.is_none());

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn attempt_acknowledgement_append_failure_blocks_following_terminal_confirmation() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-attempt-ack-failure-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let attempt = test_attempt_snapshot();
        let record =
            crate::tests::test_proxy_capture_record(&attempt.invoke_id, &attempt.occurred_at);
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        assert!(
            journal.append_attempt_start(&attempt).sequence.is_some(),
            "attempt start must be accepted before injecting the append failure"
        );
        journal.current_file = File::open(&root).expect("open directory as failed append target");

        assert!(!journal.acknowledge_attempt_start(&attempt.attempt_public_id));
        let outcome = journal.append_with_attempts(&record, &[attempt], false, None);
        assert_eq!(
            outcome.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        );
        assert_eq!(outcome.sequence, None);
        assert!(journal.append_failed);

        drop(journal);
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn unmatched_attempt_start_is_recovered_as_interrupted_terminal_event() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-attempt-interrupted-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let attempt = test_attempt_snapshot();
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append_attempt_start(&attempt);
        journal.force_sync().expect("sync start event");
        drop(journal);

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        let write = recovered
            .take_deferred_writes(1)
            .pop()
            .expect("recovered interrupted terminal write");
        assert_eq!(write.record.status, crate::INVOCATION_STATUS_INTERRUPTED);
        assert_eq!(
            write.record.failure_kind.as_deref(),
            Some(crate::PROXY_FAILURE_INVOCATION_INTERRUPTED)
        );
        assert_eq!(write.attempts.len(), 1);
        assert_eq!(
            write.attempts[0].status,
            crate::POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_INTERRUPTED
        );
        assert_eq!(
            write.attempts[0].failure_kind.as_deref(),
            Some(crate::PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
        );

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn unsynced_attempt_start_is_replayed_after_close_and_reopen() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-unsynced-start-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let attempt = test_attempt_snapshot();
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        assert_eq!(
            journal.append_attempt_start(&attempt).durability_mode,
            TerminalJournalDurabilityMode::Journal
        );
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        let write = reopened
            .take_deferred_writes(1)
            .pop()
            .expect("unsynced begin event should survive close and reopen");
        assert_eq!(write.attempts.len(), 1);
        assert_eq!(
            write.attempts[0].attempt_public_id,
            attempt.attempt_public_id
        );
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

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
        assert!(replayed.stats().oldest_pending_age_ms >= 60_000);
        let replay = replayed.take_replay();
        assert_eq!(replay.len(), 1);
        let replayed_capture_started = replay[0]
            .capture_started
            .expect("replay should preserve raw capture timing");
        assert!(replayed_capture_started.elapsed() >= Duration::from_millis(50));
        replayed.acknowledge("journal-1", "2026-07-29T00:00:00Z", false);
        replayed.force_sync().expect("sync acknowledgement");
        assert_eq!(replayed.stats().pending_records, 0);
        assert!(
            !terminal_journal_directory(&database_path)
                .join("shutdown-terminal-quarantine.jsonl")
                .exists()
        );
        drop(replayed);

        let replayed_after_ack =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert_eq!(replayed_after_ack.stats().replay_count, 0);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn replay_capacity_transfer_does_not_duplicate_obligation() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-replay-transfer-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record = crate::tests::test_proxy_capture_record(
            "journal-replay-transfer",
            "2026-07-29T00:00:00Z",
        );
        let terminal = BatchedTerminalInvocationWrite {
            enqueued_at: None,
            diagnostic: None,
            record: record.clone(),
            attempts: Vec::new(),
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        };

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&record, false, None);
        journal.force_sync().expect("sync journal");
        drop(journal);

        let mut journal = TerminalJournal::open(&database_path).expect("reopen journal");
        for _ in 0..TERMINAL_JOURNAL_MAX_DEFERRED_WRITES {
            assert!(journal.defer_write(terminal.clone()));
        }
        assert_eq!(journal.stats().replay_count, 1);
        journal.queue_replay_for_dispatch(usize::MAX);
        assert_eq!(journal.stats().replay_count, 1);
        assert_eq!(journal.shutdown_recovery_pending.len(), 1);

        journal.acknowledge(&record.invoke_id, &record.occurred_at, false);
        assert_eq!(journal.stats().replay_count, 0);
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn replay_segment_failure_marks_journal_unavailable() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-replay-unavailable-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record = crate::tests::test_proxy_capture_record(
            "journal-replay-unavailable",
            "2026-07-29T00:00:00Z",
        );
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&record, false, None);
        journal.force_sync().expect("sync journal");
        drop(journal);
        let mut journal = TerminalJournal::open(&database_path).expect("reopen journal");
        let first_sequence = journal
            .replay_segments
            .front()
            .expect("replay segment")
            .first_sequence;
        let segment_path = journal
            .segments
            .get(&first_sequence)
            .expect("replay segment metadata")
            .path
            .clone();
        fs::remove_file(segment_path).expect("remove replay segment");

        assert!(journal.take_replay().is_empty());
        assert!(journal.stats().unavailable);
        let new_record = crate::tests::test_proxy_capture_record(
            "journal-replay-unavailable-new",
            "2026-07-29T00:01:00Z",
        );
        assert_eq!(
            journal.append(&new_record, false, None).durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        );
        assert!(journal.sequence_is_synced(1).is_err());
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn replay_revalidates_checksum_after_journal_open() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-replay-checksum-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record = crate::tests::test_proxy_capture_record(
            "journal-replay-checksum",
            "2026-07-29T00:00:00Z",
        );
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&record, false, None);
        journal.force_sync().expect("sync journal");
        drop(journal);

        let mut reopened = TerminalJournal::open(&database_path).expect("reopen journal");
        let segment_path = reopened
            .segments
            .values()
            .find(|segment| !segment.current && segment.bytes > 0)
            .or_else(|| reopened.segments.values().find(|segment| segment.bytes > 0))
            .expect("replay segment")
            .path
            .clone();
        let mut bytes = fs::read(&segment_path).expect("read replay segment");
        let marker = b"\"checksum\":\"";
        let checksum_offset = bytes
            .windows(marker.len())
            .position(|window| window == marker)
            .map(|offset| offset + marker.len())
            .expect("checksum field");
        bytes[checksum_offset] = if bytes[checksum_offset] == b'0' {
            b'1'
        } else {
            b'0'
        };
        fs::write(&segment_path, bytes).expect("tamper replay checksum");

        assert!(reopened.take_replay().is_empty());
        let stats = reopened.stats();
        assert!(stats.unavailable);
        assert_eq!(stats.replay_count, 1);
        drop(reopened);
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn journal_does_not_persist_request_or_response_body_values() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-redaction-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let mut record =
            crate::tests::test_proxy_capture_record("journal-redaction-1", "2026-07-29T00:00:00Z");
        record.payload = Some("request-secret-body".to_string());
        record.raw_response = "response-secret-body".to_string();

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal
            .append(&record, false, None)
            .sequence
            .expect("append redacted terminal record");
        journal.force_sync().expect("sync journal");
        let directory = terminal_journal_directory(&database_path);
        let contents = fs::read_dir(directory)
            .expect("read journal directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("terminal-"))
            .map(|entry| fs::read_to_string(entry.path()).expect("read journal segment"))
            .collect::<String>();
        assert!(!contents.contains("request-secret-body"));
        assert!(!contents.contains("response-secret-body"));
        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn acknowledgement_clears_shadowed_shutdown_recovery() {
        let root =
            std::env::temp_dir().join(format!("terminal-journal-shadowed-{}", nanoid::nanoid!()));
        fs::create_dir_all(&root).expect("create owned fixture");
        let database_path = root.join("monitor.db");
        let record = crate::tests::test_proxy_capture_record("shadowed", "2026-07-29T00:00:00Z");
        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal.append(&record, false, None);
        journal.force_sync().expect("sync accepted terminal");
        let terminal = BatchedTerminalInvocationWrite {
            enqueued_at: None,
            diagnostic: None,
            record: record.clone(),
            attempts: Vec::new(),
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        };
        journal
            .quarantine_shutdown_batch(&[&terminal], &[], "shadowed recovery fixture")
            .expect("persist shutdown recovery before its acknowledgement");
        assert!(journal.remember_shutdown_recovery(&[&terminal]));
        journal.acknowledge(&record.invoke_id, &record.occurred_at, false);
        journal.queue_replay_for_dispatch(32);
        assert!(
            journal.take_deferred_writes(32).is_empty(),
            "a committed terminal must not be dispatched again through shutdown recovery"
        );
        assert_eq!(journal.stats().pending_materialization_records, 0);
        drop(journal);
        fs::remove_dir_all(root).expect("remove owned fixture");
    }

    #[test]
    fn shutdown_recovery_replays_and_compacts_after_acknowledgement() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-shutdown-recovery-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let record =
            crate::tests::test_proxy_capture_record("shutdown-recovery-1", "2026-07-29T00:00:00Z");
        let attempt = test_attempt_snapshot();
        let terminal = BatchedTerminalInvocationWrite {
            enqueued_at: None,
            diagnostic: None,
            record: record.clone(),
            attempts: vec![attempt.clone()],
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: Vec::new(),
        };

        let journal = TerminalJournal::open(&database_path).expect("open journal");
        TerminalJournal::quarantine_shutdown_batch_at_database_path(
            &database_path,
            &[&terminal],
            &[],
            "test recovery",
        )
        .expect("write shutdown recovery");
        drop(journal);

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen recovery journal");
        assert_eq!(recovered.stats().replay_count, 1);
        let recovered_write = recovered
            .take_deferred_writes(1)
            .pop()
            .expect("recover terminal write");
        assert_eq!(recovered_write.attempts.len(), 1);
        assert_eq!(
            recovered_write.attempts[0].attempt_public_id,
            attempt.attempt_public_id
        );
        recovered.acknowledge(&record.invoke_id, &record.occurred_at, false);
        assert_eq!(recovered.stats().replay_count, 0);
        drop(recovered);

        let recovery_path =
            terminal_journal_directory(&database_path).join("shutdown-terminal-quarantine.jsonl");
        assert!(
            fs::read_to_string(&recovery_path)
                .expect("read compacted recovery")
                .trim()
                .is_empty()
        );
        let reopened = TerminalJournal::open(&database_path).expect("reopen compacted recovery");
        assert_eq!(reopened.stats().replay_count, 0);

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn system_task_recovery_replays_across_restart_and_acknowledges() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-system-task-recovery-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let finish = BatchedSystemTaskFinish {
            run_id: 701,
            task_key: "invocation_detail_prune".to_string(),
            task_kind: SystemTaskKind::StartupBackfill,
            trigger_kind: "startup".to_string(),
            status: SystemTaskStatus::Skipped,
            summary: Some("cancelled".to_string()),
            detail: Some("shutdown requested".to_string()),
            finished_at: "2026-07-29T00:00:01Z".to_string(),
            duration_ms: 25,
            completion: Some("deferred".to_string()),
            core_completion: Some("partial".to_string()),
            details: Some(serde_json::json!({
                "ownershipVersion": 2,
                "waitReason": "schedule_retry",
                "scheduleRetryAfterSecs": 300,
                "scheduleRetryReason": "owner_failed"
            })),
        };

        let mut journal = TerminalJournal::open(&database_path).expect("open journal");
        journal
            .quarantine_system_task_finish(&finish, "test recovery")
            .expect("persist system-task recovery");
        drop(journal);

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        assert!(recovered.system_task_recovery_pending());
        let replay = recovered.take_system_task_finishes(1);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].run_id, finish.run_id);
        assert_eq!(replay[0].status, finish.status);
        assert_eq!(replay[0].task_key, finish.task_key);
        assert_eq!(replay[0].completion, finish.completion);
        assert_eq!(replay[0].core_completion, finish.core_completion);
        assert_eq!(replay[0].details, finish.details);
        recovered.defer_system_task_acknowledgements(&[finish.run_id]);
        assert_eq!(
            recovered
                .retry_system_task_acknowledgements()
                .expect("acknowledge system-task recovery"),
            1
        );
        assert!(!recovered.system_task_recovery_pending());
        assert!(recovered.take_system_task_finishes(1).is_empty());
        drop(recovered);

        let mut reopened =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert!(reopened.take_system_task_finishes(1).is_empty());

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn shutdown_system_task_recovery_replays_and_acknowledges_after_restart() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-shutdown-system-task-recovery-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let finish = BatchedSystemTaskFinish {
            run_id: 702,
            task_key: SystemTaskKind::HourlyRollupBootstrap.as_str().to_string(),
            task_kind: SystemTaskKind::HourlyRollupBootstrap,
            trigger_kind: "startup".to_string(),
            status: SystemTaskStatus::Skipped,
            summary: Some("cancelled".to_string()),
            detail: None,
            finished_at: "2026-07-29T00:00:02Z".to_string(),
            duration_ms: 40,
            completion: None,
            core_completion: None,
            details: None,
        };
        TerminalJournal::quarantine_shutdown_batch_at_database_path(
            &database_path,
            &[],
            &[&finish],
            "shutdown recovery",
        )
        .expect("persist shutdown system-task recovery");

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(recovered.take_system_task_finishes(1).len(), 1);
        recovered
            .acknowledge_system_task_finishes(&[finish.run_id])
            .expect("acknowledge shutdown system-task recovery");
        drop(recovered);

        let mut reopened =
            TerminalJournal::open(&database_path).expect("reopen acknowledged journal");
        assert!(reopened.take_system_task_finishes(1).is_empty());

        fs::remove_dir_all(root).expect("remove journal test directory");
    }

    #[test]
    fn shutdown_system_task_ack_survives_regular_quarantine_compaction() {
        let root = std::env::temp_dir().join(format!(
            "terminal-journal-shutdown-system-task-compaction-{}",
            nanoid::nanoid!()
        ));
        fs::create_dir_all(&root).expect("create journal test directory");
        let database_path = root.join("monitor.db");
        let finish = BatchedSystemTaskFinish {
            run_id: 702,
            task_key: SystemTaskKind::HourlyRollupBootstrap.as_str().to_string(),
            task_kind: SystemTaskKind::HourlyRollupBootstrap,
            trigger_kind: "startup".to_string(),
            status: SystemTaskStatus::Skipped,
            summary: Some("cancelled".to_string()),
            detail: None,
            finished_at: "2026-07-29T00:00:02Z".to_string(),
            duration_ms: 40,
            completion: None,
            core_completion: None,
            details: None,
        };
        TerminalJournal::quarantine_shutdown_batch_at_database_path(
            &database_path,
            &[],
            &[&finish],
            "shutdown recovery",
        )
        .expect("persist shutdown system-task recovery");

        let directory = terminal_journal_directory(&database_path);
        let ordinary_path = directory.join("system-task-quarantine.jsonl");
        let mut ordinary = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ordinary_path)
            .expect("open ordinary system-task quarantine");
        for run_id in 10_000_i64..130_000_i64 {
            serde_json::to_writer(
                &mut ordinary,
                &serde_json::json!({ "run_id": run_id, "acknowledged": true }),
            )
            .expect("encode ordinary acknowledgement filler");
            ordinary
                .write_all(b"\n")
                .expect("append ordinary acknowledgement filler");
        }
        ordinary
            .sync_data()
            .expect("sync ordinary acknowledgement filler");
        drop(ordinary);
        assert!(
            fs::metadata(&ordinary_path)
                .expect("stat ordinary system-task quarantine")
                .len()
                >= SYSTEM_TASK_QUARANTINE_COMPACTION_BYTES
        );

        let mut recovered = TerminalJournal::open(&database_path).expect("reopen journal");
        assert_eq!(
            recovered
                .take_system_task_finishes(1)
                .first()
                .map(|finish| finish.run_id),
            Some(finish.run_id)
        );
        recovered
            .acknowledge_system_task_finishes(&[finish.run_id])
            .expect("acknowledge shutdown system-task recovery");
        drop(recovered);

        let shutdown_path = directory.join("shutdown-system-task-quarantine.jsonl");
        assert!(
            fs::read_to_string(&shutdown_path)
                .expect("read compacted shutdown recovery")
                .trim()
                .is_empty()
        );
        let reopened = TerminalJournal::open(&database_path).expect("reopen compacted journal");
        assert!(
            reopened
                .deferred_system_task_finishes
                .iter()
                .all(|entry| entry.run_id != finish.run_id)
        );

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
