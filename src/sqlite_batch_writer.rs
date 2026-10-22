use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use sqlx::{Pool, Sqlite, SqliteConnection};
use tokio::{
    sync::{Mutex, Notify, RwLock, mpsc, oneshot},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, sleep, timeout, timeout_at},
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::*;

mod attempt_progress;
mod batch;
mod diagnostics;
mod flush;
mod invocation_identity;
mod queue;
mod queue_accounting;
mod runtime;
mod terminal_attempts;
mod writer;

use crate::terminal_journal::{
    TerminalJournal, TerminalJournalAppendOutcome, TerminalJournalDurabilityMode,
    TerminalJournalStats,
};
pub(crate) use attempt_progress::BatchedAttemptProgress;
use attempt_progress::{
    drain_reliable_attempt_progress_overflow,
    estimated_memory_bytes as estimated_attempt_progress_memory_bytes,
    merge as merge_attempt_progress,
};
use diagnostics::{append_terminal_journal, finish_committed_batch, reference_batch};
pub(crate) use invocation_identity::TrackedTerminalJournal;
use terminal_attempts::persist_runtime_attempt_snapshot_tx;

pub(crate) use batch::*;
pub(crate) use flush::*;
pub(crate) use queue::*;
pub(crate) use runtime::*;

#[derive(Debug)]
pub(crate) struct SqliteBatchWriter {
    write_sender: mpsc::Sender<SqliteBatchWrite>,
    attempt_progress_sequence: AtomicU64,
    attempt_progress_send_gate: std::sync::Mutex<()>,
    reliable_attempt_progress_overflow: Arc<std::sync::Mutex<PendingBatch>>,
    reliable_attempt_progress_notify: Arc<Notify>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
    control_sender: mpsc::Sender<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    dropped_writes: Arc<AtomicU64>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache:
        Arc<std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>>,
    summary_delta_hub: Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    terminal_journal: Arc<TrackedTerminalJournal>,
    database_path: std::path::PathBuf,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    #[cfg(test)]
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    journal_sync_shutdown: CancellationToken,
    journal_sync_handle: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    buffered_writes: Option<Arc<std::sync::Mutex<Vec<SqliteBatchWrite>>>>,
    #[cfg(test)]
    auto_flush_terminal_for_test: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    runtime_terminal_confirmation_count: AtomicUsize,
    #[cfg(test)]
    runtime_terminal_confirmation_gate: Mutex<()>,
}

#[cfg(test)]
#[path = "tests/lightweight/terminal_backfill_obligations.rs"]
mod terminal_backfill_obligations;

#[cfg(test)]
mod tests;
