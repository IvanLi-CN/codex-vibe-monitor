use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Error;
use once_cell::sync::Lazy;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::warn;

const DEFAULT_BACKGROUND_DB_SLOTS: usize = 1;
const DEFAULT_PRESSURE_COOLDOWN: Duration = Duration::from_secs(30);

static GLOBAL_DB_PRESSURE_GATE: Lazy<DbPressureGate> =
    Lazy::new(|| DbPressureGate::new(DEFAULT_BACKGROUND_DB_SLOTS, DEFAULT_PRESSURE_COOLDOWN));

pub(crate) fn global_db_pressure_gate() -> &'static DbPressureGate {
    &GLOBAL_DB_PRESSURE_GATE
}

#[derive(Debug)]
pub(crate) struct DbPressureGate {
    background_slots: Arc<Semaphore>,
    pressure_cooldown: Duration,
    pressure_until_epoch_ms: AtomicU64,
    pressure_events: AtomicU64,
    background_skips: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DbPressureDenyReason {
    PressureCooldown { remaining_ms: u64 },
    BackgroundBusy,
}

impl fmt::Display for DbPressureDenyReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PressureCooldown { remaining_ms } => {
                write!(f, "pressure_cooldown:{remaining_ms}ms")
            }
            Self::BackgroundBusy => f.write_str("background_busy"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct DbBackgroundPermit {
    _permit: OwnedSemaphorePermit,
    started_at: Instant,
}

impl DbBackgroundPermit {
    #[allow(dead_code)]
    pub(crate) fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct DbPressureSnapshot {
    pub(crate) pressure_cooldown_remaining_ms: u64,
    pub(crate) pressure_events: u64,
    pub(crate) background_skips: u64,
}

impl DbPressureGate {
    pub(crate) fn new(background_slots: usize, pressure_cooldown: Duration) -> Self {
        Self {
            background_slots: Arc::new(Semaphore::new(background_slots.max(1))),
            pressure_cooldown,
            pressure_until_epoch_ms: AtomicU64::new(0),
            pressure_events: AtomicU64::new(0),
            background_skips: AtomicU64::new(0),
        }
    }

    pub(crate) fn try_begin_background(
        &self,
        _task: &'static str,
    ) -> Result<DbBackgroundPermit, DbPressureDenyReason> {
        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }

        let permit = self
            .background_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                self.background_skips.fetch_add(1, Ordering::Relaxed);
                DbPressureDenyReason::BackgroundBusy
            })?;

        Ok(DbBackgroundPermit {
            _permit: permit,
            started_at: Instant::now(),
        })
    }

    pub(crate) fn record_error(&self, task: &'static str, err: &Error) -> bool {
        if !is_db_pressure_error(err) {
            return false;
        }
        self.record_pressure(task, "sqlite_or_pool_pressure");
        true
    }

    pub(crate) fn record_pressure(&self, task: &'static str, reason: &'static str) {
        let now_ms = current_epoch_ms();
        let cooldown_ms = duration_ms_u64(self.pressure_cooldown);
        let until_ms = now_ms.saturating_add(cooldown_ms);
        update_atomic_max(&self.pressure_until_epoch_ms, until_ms);
        let events = self.pressure_events.fetch_add(1, Ordering::Relaxed) + 1;
        warn!(
            task,
            reason,
            events,
            cooldown_ms,
            "database pressure detected; background database work will back off"
        );
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn snapshot(&self) -> DbPressureSnapshot {
        let now_ms = current_epoch_ms();
        DbPressureSnapshot {
            pressure_cooldown_remaining_ms: self
                .pressure_until_epoch_ms
                .load(Ordering::Acquire)
                .saturating_sub(now_ms),
            pressure_events: self.pressure_events.load(Ordering::Relaxed),
            background_skips: self.background_skips.load(Ordering::Relaxed),
        }
    }
}

pub(crate) fn is_db_pressure_error(err: &Error) -> bool {
    crate::is_sqlite_lock_error(err) || is_pool_acquire_timeout_error(err)
}

fn is_pool_acquire_timeout_error(err: &Error) -> bool {
    err.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("pool timed out")
            || message.contains("timed out while waiting for an open connection")
    })
}

fn update_atomic_max(value: &AtomicU64, candidate: u64) {
    let mut current = value.load(Ordering::Acquire);
    while candidate > current {
        match value.compare_exchange(current, candidate, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

fn duration_ms_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn gate_denies_background_during_pressure_cooldown() {
        let gate = DbPressureGate::new(1, Duration::from_secs(60));
        gate.record_pressure("test", "forced");

        let denied = gate.try_begin_background("maintenance").unwrap_err();
        assert!(matches!(
            denied,
            DbPressureDenyReason::PressureCooldown { remaining_ms } if remaining_ms > 0
        ));
        assert_eq!(gate.snapshot().pressure_events, 1);
        assert_eq!(gate.snapshot().background_skips, 1);
    }

    #[test]
    fn gate_singleflights_background_work() {
        let gate = DbPressureGate::new(1, Duration::from_secs(1));
        let permit = gate
            .try_begin_background("first")
            .expect("first background permit");

        assert_eq!(
            gate.try_begin_background("second").unwrap_err(),
            DbPressureDenyReason::BackgroundBusy
        );

        drop(permit);
        assert!(gate.try_begin_background("second").is_ok());
    }

    #[test]
    fn db_pressure_error_detects_pool_acquire_timeout() {
        let err = anyhow!("pool timed out while waiting for an open connection");
        assert!(is_db_pressure_error(&err));
    }
}
