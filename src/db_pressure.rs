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
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tracing::warn;

const DEFAULT_BACKGROUND_DB_SLOTS: usize = 1;
const DEFAULT_PRESSURE_COOLDOWN: Duration = Duration::from_secs(30);
const BACKGROUND_BUSY_WAIT_POLL: Duration = Duration::from_millis(25);

static GLOBAL_DB_PRESSURE_GATE: Lazy<DbPressureGate> = Lazy::new(|| {
    DbPressureGate::new_global(DEFAULT_BACKGROUND_DB_SLOTS, DEFAULT_PRESSURE_COOLDOWN)
});

pub(crate) fn global_db_pressure_gate() -> &'static DbPressureGate {
    &GLOBAL_DB_PRESSURE_GATE
}

#[derive(Debug)]
pub(crate) struct DbPressureGate {
    background_slots: Arc<Semaphore>,
    priority_waiters: Arc<AtomicU64>,
    pressure_cooldown: Duration,
    pressure_until_epoch_ms: AtomicU64,
    pressure_events: AtomicU64,
    background_skips: AtomicU64,
    eligibility: Arc<DbPressureEligibility>,
    #[cfg(test)]
    bypass_for_test_global: bool,
}

#[derive(Debug, Default)]
struct DbPressureEligibility {
    generation: AtomicU64,
    notify: Notify,
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
    _permit: Option<OwnedSemaphorePermit>,
    started_at: Instant,
    eligibility: Option<Arc<DbPressureEligibility>>,
}

#[derive(Debug)]
struct DbBackgroundPriorityWaiter {
    priority_waiters: Arc<AtomicU64>,
}

impl DbBackgroundPriorityWaiter {
    fn register(priority_waiters: Arc<AtomicU64>) -> Self {
        priority_waiters.fetch_add(1, Ordering::AcqRel);
        Self { priority_waiters }
    }
}

impl Drop for DbBackgroundPriorityWaiter {
    fn drop(&mut self) {
        self.priority_waiters.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Reserves the next background admission for a bounded critical recovery turn.
///
/// The reservation is intentionally held before the recovery worker is scheduled: a long
/// best-effort task must not win the only slot merely because it happened to start during the
/// short gap between HTTP readiness and the worker's first poll.
#[derive(Debug)]
pub(crate) struct DbBackgroundPriorityReservation {
    _waiter: Option<DbBackgroundPriorityWaiter>,
}

impl Drop for DbBackgroundPermit {
    fn drop(&mut self) {
        self._permit.take();
        if let Some(eligibility) = &self.eligibility {
            eligibility.generation.fetch_add(1, Ordering::AcqRel);
            eligibility.notify.notify_waiters();
        }
    }
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
            priority_waiters: Arc::new(AtomicU64::new(0)),
            pressure_cooldown,
            pressure_until_epoch_ms: AtomicU64::new(0),
            pressure_events: AtomicU64::new(0),
            background_skips: AtomicU64::new(0),
            eligibility: Arc::new(DbPressureEligibility::default()),
            #[cfg(test)]
            bypass_for_test_global: false,
        }
    }

    fn new_global(background_slots: usize, pressure_cooldown: Duration) -> Self {
        let gate = Self::new(background_slots, pressure_cooldown);
        #[cfg(test)]
        {
            Self {
                bypass_for_test_global: true,
                ..gate
            }
        }
        #[cfg(not(test))]
        {
            gate
        }
    }

    pub(crate) fn background_deny_reason(&self) -> Option<DbPressureDenyReason> {
        #[cfg(test)]
        if self.bypass_for_test_global {
            return None;
        }

        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            return Some(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }
        if self.background_slots.available_permits() == 0 {
            return Some(DbPressureDenyReason::BackgroundBusy);
        }
        None
    }

    pub(crate) fn pressure_cooldown_deadline_epoch_ms(&self) -> Option<u64> {
        let now_ms = current_epoch_ms();
        let deadline_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        (deadline_ms > now_ms).then_some(deadline_ms)
    }

    pub(crate) fn try_begin_background(
        &self,
        _task: &'static str,
    ) -> Result<DbBackgroundPermit, DbPressureDenyReason> {
        #[cfg(test)]
        if self.bypass_for_test_global {
            return Ok(DbBackgroundPermit {
                _permit: None,
                started_at: Instant::now(),
                eligibility: None,
            });
        }

        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }
        if self.priority_waiters.load(Ordering::Acquire) > 0 {
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::BackgroundBusy);
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
            _permit: Some(permit),
            started_at: Instant::now(),
            eligibility: Some(self.eligibility.clone()),
        })
    }

    pub(crate) async fn begin_background_with_busy_wait(
        &self,
        _task: &'static str,
        max_wait: Duration,
    ) -> Result<DbBackgroundPermit, DbPressureDenyReason> {
        #[cfg(test)]
        if self.bypass_for_test_global {
            return Ok(DbBackgroundPermit {
                _permit: None,
                started_at: Instant::now(),
                eligibility: None,
            });
        }

        let started_at = Instant::now();
        loop {
            let now_ms = current_epoch_ms();
            let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
            if pressure_until_ms > now_ms {
                self.background_skips.fetch_add(1, Ordering::Relaxed);
                return Err(DbPressureDenyReason::PressureCooldown {
                    remaining_ms: pressure_until_ms.saturating_sub(now_ms),
                });
            }
            if self.priority_waiters.load(Ordering::Acquire) > 0 {
                self.background_skips.fetch_add(1, Ordering::Relaxed);
                return Err(DbPressureDenyReason::BackgroundBusy);
            }

            if let Ok(permit) = self.background_slots.clone().try_acquire_owned() {
                return Ok(DbBackgroundPermit {
                    _permit: Some(permit),
                    started_at: Instant::now(),
                    eligibility: Some(self.eligibility.clone()),
                });
            }

            let elapsed = started_at.elapsed();
            if elapsed >= max_wait {
                self.background_skips.fetch_add(1, Ordering::Relaxed);
                return Err(DbPressureDenyReason::BackgroundBusy);
            }
            let remaining = max_wait.saturating_sub(elapsed);
            tokio::time::sleep(remaining.min(BACKGROUND_BUSY_WAIT_POLL)).await;
        }
    }

    /// Waits for a bounded turn in the semaphore's FIFO queue. While queued, new best-effort
    /// background work cannot overtake the recovery worker at the next permit release.
    pub(crate) async fn begin_priority_background_with_queue_wait(
        &self,
        _task: &'static str,
        max_wait: Duration,
    ) -> Result<DbBackgroundPermit, DbPressureDenyReason> {
        let reservation = self.reserve_priority_background();
        self.begin_reserved_priority_background(reservation, max_wait)
            .await
    }

    /// Prevents new best-effort background work from overtaking a recovery worker before that
    /// worker begins waiting for the sole database slot.
    pub(crate) fn reserve_priority_background(&self) -> DbBackgroundPriorityReservation {
        #[cfg(test)]
        if self.bypass_for_test_global {
            return DbBackgroundPriorityReservation { _waiter: None };
        }

        DbBackgroundPriorityReservation {
            _waiter: Some(DbBackgroundPriorityWaiter::register(
                self.priority_waiters.clone(),
            )),
        }
    }

    /// Consumes a pre-registered reservation and waits for the next eligible background slot.
    /// The reservation remains held through the wait and is released only after the permit has
    /// been acquired (or the operation has been rejected), so best-effort work cannot overtake
    /// this recovery turn.
    pub(crate) async fn begin_reserved_priority_background(
        &self,
        reservation: DbBackgroundPriorityReservation,
        max_wait: Duration,
    ) -> Result<DbBackgroundPermit, DbPressureDenyReason> {
        #[cfg(test)]
        if self.bypass_for_test_global {
            drop(reservation);
            return Ok(DbBackgroundPermit {
                _permit: None,
                started_at: Instant::now(),
                eligibility: None,
            });
        }

        let started_at = Instant::now();
        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            drop(reservation);
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }

        let permit =
            match tokio::time::timeout(max_wait, self.background_slots.clone().acquire_owned())
                .await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) | Err(_) => {
                    drop(reservation);
                    self.background_skips.fetch_add(1, Ordering::Relaxed);
                    return Err(DbPressureDenyReason::BackgroundBusy);
                }
            };

        drop(reservation);

        // A pressure event can occur while this task is queued. Do not begin progress after a
        // cooldown has started; dropping the permit wakes the next eligible worker.
        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            drop(permit);
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }

        Ok(DbBackgroundPermit {
            _permit: Some(permit),
            started_at,
            eligibility: Some(self.eligibility.clone()),
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
        self.eligibility.generation.fetch_add(1, Ordering::AcqRel);
        self.eligibility.notify.notify_waiters();
        warn!(
            task,
            reason,
            events,
            cooldown_ms,
            "database pressure detected; background database work will back off"
        );
    }

    pub(crate) fn eligibility_generation(&self) -> u64 {
        self.eligibility.generation.load(Ordering::Acquire)
    }

    pub(crate) fn notify_background_eligibility(&self) {
        self.eligibility.generation.fetch_add(1, Ordering::AcqRel);
        self.eligibility.notify.notify_waiters();
    }

    pub(crate) async fn wait_for_eligibility_change(&self, observed: u64) {
        loop {
            let notified = self.eligibility.notify.notified();
            if self.eligibility_generation() != observed {
                return;
            }
            notified.await;
        }
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
    fn cooldown_deadline_stays_stable_across_denials() {
        let gate = DbPressureGate::new(1, Duration::from_secs(60));
        gate.record_pressure("test", "forced");

        let deadline = gate
            .pressure_cooldown_deadline_epoch_ms()
            .expect("active pressure cooldown deadline");
        assert!(matches!(
            gate.try_begin_background("first"),
            Err(DbPressureDenyReason::PressureCooldown { .. })
        ));
        assert!(matches!(
            gate.try_begin_background("second"),
            Err(DbPressureDenyReason::PressureCooldown { .. })
        ));
        assert_eq!(
            gate.pressure_cooldown_deadline_epoch_ms(),
            Some(deadline),
            "a cooldown must keep one absolute next-eligibility deadline"
        );
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

    #[tokio::test]
    async fn gate_busy_waits_for_background_slot_release() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let permit = gate
            .try_begin_background("first")
            .expect("first background permit");
        let waiter_gate = gate.clone();
        let waiter = tokio::spawn(async move {
            waiter_gate
                .begin_background_with_busy_wait("second", Duration::from_secs(1))
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !waiter.is_finished(),
            "waiter should stay pending while the slot is busy"
        );

        drop(permit);
        let second = waiter
            .await
            .expect("waiter task should not panic")
            .expect("second background permit");
        drop(second);
    }

    #[tokio::test]
    async fn gate_queue_wait_claims_the_next_released_background_slot() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let first = gate
            .try_begin_background("first")
            .expect("first background permit");
        let waiter_gate = gate.clone();
        let waiter = tokio::spawn(async move {
            waiter_gate
                .begin_priority_background_with_queue_wait("recovery", Duration::from_secs(1))
                .await
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !waiter.is_finished(),
            "queued recovery must wait while another background page owns the slot"
        );

        drop(first);
        let recovery = waiter
            .await
            .expect("queued recovery task should not panic")
            .expect("queued recovery should receive the released slot");
        assert_eq!(
            gate.try_begin_background("best_effort").unwrap_err(),
            DbPressureDenyReason::BackgroundBusy,
            "a newly arriving best-effort task must not overtake queued recovery"
        );
        drop(recovery);
    }

    #[tokio::test]
    async fn priority_reservation_blocks_best_effort_before_recovery_waits() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let first = gate
            .try_begin_background("first")
            .expect("first background permit");
        let reservation = gate.reserve_priority_background();

        assert_eq!(
            gate.try_begin_background("best_effort").unwrap_err(),
            DbPressureDenyReason::BackgroundBusy,
            "a startup reservation must fence best-effort admission before its worker polls"
        );

        let recovery_gate = gate.clone();
        let recovery = tokio::spawn(async move {
            recovery_gate
                .begin_reserved_priority_background(reservation, Duration::from_secs(1))
                .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !recovery.is_finished(),
            "the reserved recovery must wait only for the already-running page"
        );

        drop(first);
        let permit = recovery
            .await
            .expect("recovery task should not panic")
            .expect("reserved recovery should receive the released slot");
        drop(permit);
    }

    #[tokio::test]
    async fn chained_priority_reservations_keep_generic_work_between_recovery_pages_out() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let first_reservation = gate.reserve_priority_background();
        let first = gate
            .begin_reserved_priority_background(first_reservation, Duration::from_secs(1))
            .await
            .expect("first recovery page admission");

        // The next reservation is created while the first page still owns the slot. Once that
        // page commits and releases, a generic worker must not fill the inter-page gap.
        let next_reservation = gate.reserve_priority_background();
        drop(first);
        assert_eq!(
            gate.try_begin_background("best_effort").unwrap_err(),
            DbPressureDenyReason::BackgroundBusy,
            "a queued recovery continuation must own the next admission"
        );
        let second = gate
            .begin_reserved_priority_background(next_reservation, Duration::from_secs(1))
            .await
            .expect("chained recovery page admission");
        drop(second);
    }

    #[tokio::test]
    async fn gate_notifies_eligibility_generation_when_slot_is_released() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let permit = gate
            .try_begin_background("first")
            .expect("first background permit");
        let observed = gate.eligibility_generation();
        let waiter_gate = gate.clone();
        let waiter = tokio::spawn(async move {
            waiter_gate.wait_for_eligibility_change(observed).await;
            waiter_gate.eligibility_generation()
        });

        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        drop(permit);
        let next_generation = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("eligibility waiter should wake")
            .expect("eligibility waiter should not panic");
        assert!(next_generation > observed);
    }

    #[tokio::test]
    async fn eligibility_wait_observes_release_that_precedes_wait_registration() {
        let gate = DbPressureGate::new(1, Duration::from_secs(1));
        let permit = gate
            .try_begin_background("first")
            .expect("first background permit");
        let observed = gate.eligibility_generation();
        assert_eq!(
            gate.try_begin_background("second").unwrap_err(),
            DbPressureDenyReason::BackgroundBusy
        );

        drop(permit);

        tokio::time::timeout(
            Duration::from_millis(50),
            gate.wait_for_eligibility_change(observed),
        )
        .await
        .expect("release before waiter registration must still be observed");
    }

    #[tokio::test]
    async fn gate_busy_wait_does_not_wait_through_pressure_cooldown() {
        let gate = DbPressureGate::new(1, Duration::from_secs(60));
        gate.record_pressure("test", "forced");
        let started = Instant::now();

        let denied = gate
            .begin_background_with_busy_wait("maintenance", Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(matches!(
            denied,
            DbPressureDenyReason::PressureCooldown { remaining_ms } if remaining_ms > 0
        ));
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "pressure cooldown should fail fast instead of consuming the busy wait budget"
        );
    }

    #[test]
    fn global_gate_bypasses_background_limits_in_tests() {
        let gate = global_db_pressure_gate();
        let first = gate
            .try_begin_background("first")
            .expect("first background permit");

        assert!(gate.try_begin_background("second").is_ok());

        drop(first);
    }

    #[test]
    fn db_pressure_error_detects_pool_acquire_timeout() {
        let err = anyhow!("pool timed out while waiting for an open connection");
        assert!(is_db_pressure_error(&err));
    }
}
