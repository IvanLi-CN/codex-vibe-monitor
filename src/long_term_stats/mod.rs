use super::*;
use sha2::{Digest, Sha256};

#[path = "projection_flush.rs"]
mod projection_flush;
use projection_flush::flush_long_term_projection;

const LONG_TERM_TIMEZONE: &str = "Asia/Shanghai";
const LONG_TERM_STATE_ID: i64 = 1;
const LONG_TERM_STATUS_DISABLED: &str = "disabled";
const LONG_TERM_STATUS_PREPARING: &str = "preparing";
const LONG_TERM_STATUS_RUNNING: &str = "running";
const LONG_TERM_STATUS_READY: &str = "ready";
const LONG_TERM_STATUS_EMPTY: &str = "empty";
const LONG_TERM_STATUS_ERROR: &str = "error";
const LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR: &str =
    "long-term initial materialization is incomplete";
const LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR: &str =
    "terminal integrity proof reconciliation is incomplete";
const LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR: &str =
    "attempt archive is unavailable for long-term account attribution";
const LONG_TERM_OTHER_KEY: &str = "other";
const LONG_TERM_OTHER_NAME: &str = "其他";
const LONG_TERM_HOUR_MS: i64 = 60 * 60 * 1000;
const LONG_TERM_INTEGRITY_AUDIT_INTERVAL_SECS: i64 = 60 * 60;
const LONG_TERM_REPAIR_BACKOFF_SECS: [i64; 4] = [60, 5 * 60, 15 * 60, 60 * 60];
const LONG_TERM_REFRESH_LOCK_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(3),
];
const LONG_TERM_PROJECTION_LOCK_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(3),
];
pub(crate) const LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET: &str = "long_term_usage_stats";

static LONG_TERM_REFRESH_LOCK: once_cell::sync::Lazy<Mutex<()>> =
    once_cell::sync::Lazy::new(|| Mutex::new(()));
static LONG_TERM_PROJECTION_PUBLICATION_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

mod model;
pub(crate) use model::*;

mod archive;
use archive::*;

mod schema;
pub(crate) use schema::*;

mod projection_runtime;
pub(crate) use projection_runtime::*;

mod projection_rebuild;
use projection_rebuild::*;

mod integrity;
pub(crate) use integrity::*;

mod refresh;
pub(crate) use refresh::*;

mod intervals;
pub(crate) use intervals::*;

#[cfg(test)]
#[path = "../tests/stateful_sqlite/statistics_observability.rs"]
mod statistics_observability;
#[cfg(test)]
mod tests;
