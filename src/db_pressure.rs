use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread::ThreadId,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Error;
use once_cell::sync::Lazy;
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
    task::Id as TaskId,
};
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
    pressure_cooldown: Duration,
    pressure_until_epoch_ms: AtomicU64,
    pressure_generation: AtomicU64,
    pressure_events: AtomicU64,
    background_skips: AtomicU64,
    eligibility: Arc<DbPressureEligibility>,
    active_admissions: Arc<Mutex<HashMap<DbBackgroundAdmissionOwner, Weak<DbBackgroundAdmission>>>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DbBackgroundAdmissionOwner {
    TokioTask(TaskId),
    RuntimeRoot(ThreadId),
}

#[derive(Debug)]
struct DbBackgroundAdmission {
    permit: Option<OwnedSemaphorePermit>,
    eligibility: Arc<DbPressureEligibility>,
    owner: Option<DbBackgroundAdmissionOwner>,
    active_admissions: Arc<Mutex<HashMap<DbBackgroundAdmissionOwner, Weak<DbBackgroundAdmission>>>>,
}

impl Drop for DbBackgroundAdmission {
    fn drop(&mut self) {
        self.permit.take();
        if let Some(owner) = self.owner
            && let Ok(mut admissions) = self.active_admissions.lock()
        {
            admissions.remove(&owner);
        }
        self.eligibility.generation.fetch_add(1, Ordering::AcqRel);
        self.eligibility.notify.notify_waiters();
    }
}

#[derive(Debug)]
pub(crate) struct DbBackgroundPermit {
    admission: Option<Arc<DbBackgroundAdmission>>,
    started_at: Instant,
}

impl DbBackgroundPermit {
    fn bypassed() -> Self {
        Self {
            admission: None,
            started_at: Instant::now(),
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
            pressure_cooldown,
            pressure_until_epoch_ms: AtomicU64::new(0),
            pressure_generation: AtomicU64::new(0),
            pressure_events: AtomicU64::new(0),
            background_skips: AtomicU64::new(0),
            eligibility: Arc::new(DbPressureEligibility::default()),
            active_admissions: Arc::new(Mutex::new(HashMap::new())),
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
            return Ok(DbBackgroundPermit::bypassed());
        }

        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }

        if let Some(admission) = self.reenter_current_task() {
            return Ok(admission);
        }

        let permit = self
            .background_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                self.background_skips.fetch_add(1, Ordering::Relaxed);
                DbPressureDenyReason::BackgroundBusy
            })?;
        let admission = self.new_admission(permit);

        // A pressure event can race with the pre-acquisition cooldown check. Re-check while
        // retaining the slot so a just-closed gate cannot admit another SQLite operation.
        let now_ms = current_epoch_ms();
        let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
        if pressure_until_ms > now_ms {
            drop(admission);
            self.background_skips.fetch_add(1, Ordering::Relaxed);
            return Err(DbPressureDenyReason::PressureCooldown {
                remaining_ms: pressure_until_ms.saturating_sub(now_ms),
            });
        }

        Ok(admission)
    }

    pub(crate) async fn begin_background_with_busy_wait(
        &self,
        _task: &'static str,
        max_wait: Duration,
    ) -> Result<DbBackgroundPermit, DbPressureDenyReason> {
        #[cfg(test)]
        if self.bypass_for_test_global {
            return Ok(DbBackgroundPermit::bypassed());
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

            if let Some(admission) = self.reenter_current_task() {
                return Ok(admission);
            }

            if let Ok(permit) = self.background_slots.clone().try_acquire_owned() {
                let admission = self.new_admission(permit);
                let now_ms = current_epoch_ms();
                let pressure_until_ms = self.pressure_until_epoch_ms.load(Ordering::Acquire);
                if pressure_until_ms > now_ms {
                    drop(admission);
                    self.background_skips.fetch_add(1, Ordering::Relaxed);
                    return Err(DbPressureDenyReason::PressureCooldown {
                        remaining_ms: pressure_until_ms.saturating_sub(now_ms),
                    });
                }
                return Ok(admission);
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

    fn reenter_current_task(&self) -> Option<DbBackgroundPermit> {
        let owner = current_background_admission_owner()?;
        let admission = self
            .active_admissions
            .lock()
            .ok()
            .and_then(|admissions| admissions.get(&owner).and_then(Weak::upgrade));
        admission.map(|admission| DbBackgroundPermit {
            admission: Some(admission),
            started_at: Instant::now(),
        })
    }

    fn new_admission(&self, permit: OwnedSemaphorePermit) -> DbBackgroundPermit {
        let owner = current_background_admission_owner();
        let admission = Arc::new(DbBackgroundAdmission {
            permit: Some(permit),
            eligibility: self.eligibility.clone(),
            owner,
            active_admissions: self.active_admissions.clone(),
        });
        if let Some(owner) = owner
            && let Ok(mut admissions) = self.active_admissions.lock()
        {
            admissions.insert(owner, Arc::downgrade(&admission));
        }
        DbBackgroundPermit {
            admission: Some(admission),
            started_at: Instant::now(),
        }
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
        let generation = self.pressure_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let events = self.pressure_events.fetch_add(1, Ordering::Relaxed) + 1;
        self.eligibility.generation.fetch_add(1, Ordering::AcqRel);
        self.eligibility.notify.notify_waiters();
        warn!(
            task,
            reason,
            generation,
            events,
            cooldown_ms,
            "database pressure detected; background database work will back off"
        );
    }

    pub(crate) fn eligibility_generation(&self) -> u64 {
        self.eligibility.generation.load(Ordering::Acquire)
    }

    pub(crate) fn pressure_generation(&self) -> u64 {
        self.pressure_generation.load(Ordering::Acquire)
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

fn current_background_admission_owner() -> Option<DbBackgroundAdmissionOwner> {
    tokio::task::try_id()
        .map(DbBackgroundAdmissionOwner::TokioTask)
        .or_else(|| {
            // Tokio's root future has no task ID, but it remains on the thread driving
            // `Runtime::block_on`. Restrict this fallback to an entered Tokio runtime so plain
            // synchronous callers retain normal single-flight behavior.
            tokio::runtime::Handle::try_current()
                .ok()
                .map(|_| DbBackgroundAdmissionOwner::RuntimeRoot(std::thread::current().id()))
        })
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
    fn pressure_generation_changes_only_when_new_pressure_is_recorded() {
        let gate = DbPressureGate::new(1, Duration::from_secs(60));
        let before = gate.pressure_generation();

        gate.notify_background_eligibility();
        assert_eq!(gate.pressure_generation(), before);

        gate.record_pressure("test", "forced");
        assert_eq!(gate.pressure_generation(), before + 1);
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
    async fn gate_reenters_work_held_by_a_runtime_root_future() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let outer = gate
            .try_begin_background("outer")
            .expect("root future outer background permit");
        let nested = gate
            .try_begin_background("nested")
            .expect("root future may reuse its held background permit");

        let other_gate = gate.clone();
        let other = tokio::spawn(async move { other_gate.try_begin_background("other") });
        assert_eq!(
            other
                .await
                .expect("other task should not panic")
                .unwrap_err(),
            DbPressureDenyReason::BackgroundBusy
        );

        drop(outer);
        drop(nested);
        assert!(gate.try_begin_background("after_nested").is_ok());
    }

    #[tokio::test]
    async fn gate_reenters_work_held_by_the_same_task_without_releasing_the_slot() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(1)));
        let (nested_ready_tx, nested_ready_rx) = tokio::sync::oneshot::channel();
        let (drop_outer_tx, drop_outer_rx) = tokio::sync::oneshot::channel();
        let (outer_dropped_tx, outer_dropped_rx) = tokio::sync::oneshot::channel();
        let (drop_nested_tx, drop_nested_rx) = tokio::sync::oneshot::channel();
        let worker_gate = gate.clone();
        let worker = tokio::spawn(async move {
            let outer = worker_gate
                .try_begin_background("outer")
                .expect("outer background permit");
            let nested = worker_gate
                .try_begin_background("nested")
                .expect("the current task may reuse its held background permit");
            nested_ready_tx
                .send(())
                .expect("test should await nested admission");

            drop_outer_rx
                .await
                .expect("test should request outer permit release");
            drop(outer);
            outer_dropped_tx
                .send(())
                .expect("test should await outer permit release");

            drop_nested_rx
                .await
                .expect("test should request nested permit release");
            drop(nested);
        });
        nested_ready_rx
            .await
            .expect("worker should acquire nested admission");

        let other_gate = gate.clone();
        let other = tokio::spawn(async move { other_gate.try_begin_background("other") });
        assert_eq!(
            other
                .await
                .expect("other task should not panic")
                .unwrap_err(),
            DbPressureDenyReason::BackgroundBusy
        );

        drop_outer_tx
            .send(())
            .expect("worker should be waiting to release outer permit");
        outer_dropped_rx
            .await
            .expect("worker should release outer permit");
        let other_gate = gate.clone();
        let other =
            tokio::spawn(async move { other_gate.try_begin_background("other_after_outer") });
        assert_eq!(
            other
                .await
                .expect("other task should not panic")
                .unwrap_err(),
            DbPressureDenyReason::BackgroundBusy
        );

        drop_nested_tx
            .send(())
            .expect("worker should be waiting to release nested permit");
        worker.await.expect("worker should not panic");
        assert!(gate.try_begin_background("after_nested").is_ok());
    }

    #[tokio::test]
    async fn gate_reentrant_work_observes_a_later_pressure_cooldown() {
        let gate = Arc::new(DbPressureGate::new(1, Duration::from_secs(60)));
        let (outer_ready_tx, outer_ready_rx) = tokio::sync::oneshot::channel();
        let (begin_nested_tx, begin_nested_rx) = tokio::sync::oneshot::channel();
        let worker_gate = gate.clone();
        let worker = tokio::spawn(async move {
            let _outer = worker_gate
                .try_begin_background("outer")
                .expect("outer background permit");
            outer_ready_tx
                .send(())
                .expect("test should await outer admission");
            begin_nested_rx
                .await
                .expect("test should request nested admission");
            worker_gate
                .try_begin_background("nested_after_pressure")
                .expect_err("a later pressure cooldown must deny nested SQLite work")
        });

        outer_ready_rx
            .await
            .expect("worker should acquire outer admission");
        gate.record_pressure("test", "forced");
        begin_nested_tx
            .send(())
            .expect("worker should be waiting to begin nested work");
        assert!(matches!(
            worker.await.expect("worker should not panic"),
            DbPressureDenyReason::PressureCooldown { remaining_ms } if remaining_ms > 0
        ));
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
