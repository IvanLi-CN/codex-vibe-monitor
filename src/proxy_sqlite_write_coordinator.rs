use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub(crate) const PROXY_SQLITE_WRITE_COORDINATOR_MODE_ENV: &str =
    "PROXY_SQLITE_WRITE_COORDINATOR_MODE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxySqliteWriteClass {
    P1Terminal,
    InteractiveProxy,
    P2Derived,
    MaintenanceRetention,
}

impl ProxySqliteWriteClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::P1Terminal => "p1_terminal",
            Self::InteractiveProxy => "interactive_proxy",
            Self::P2Derived => "p2_derived",
            Self::MaintenanceRetention => "maintenance_retention",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxySqliteWriteCoordinatorSnapshot {
    pub(crate) mode: String,
    pub(crate) active_write_class: Option<String>,
    pub(crate) p1_waiter_count: usize,
    pub(crate) interactive_waiter_count: usize,
    pub(crate) p2_waiter_count: usize,
    pub(crate) maintenance_waiter_count: usize,
    pub(crate) maintenance_fairness_admission_count: u64,
    pub(crate) direct_write_bypass_count: u64,
}

#[derive(Debug, Default)]
struct CoordinatorState {
    active: Option<ProxySqliteWriteClass>,
    p1_waiters: usize,
    interactive_waiters: usize,
    p2_waiters: usize,
    maintenance_waiters: usize,
    maintenance_fairness_admissions: u64,
    last_maintenance_fairness_admission: Option<Instant>,
    direct_write_bypass_count: u64,
}

impl CoordinatorState {
    fn increment(&mut self, class: ProxySqliteWriteClass) {
        match class {
            ProxySqliteWriteClass::P1Terminal => self.p1_waiters += 1,
            ProxySqliteWriteClass::InteractiveProxy => self.interactive_waiters += 1,
            ProxySqliteWriteClass::P2Derived => self.p2_waiters += 1,
            ProxySqliteWriteClass::MaintenanceRetention => self.maintenance_waiters += 1,
        }
    }

    fn decrement(&mut self, class: ProxySqliteWriteClass) {
        match class {
            ProxySqliteWriteClass::P1Terminal => self.p1_waiters -= 1,
            ProxySqliteWriteClass::InteractiveProxy => self.interactive_waiters -= 1,
            ProxySqliteWriteClass::P2Derived => self.p2_waiters -= 1,
            ProxySqliteWriteClass::MaintenanceRetention => self.maintenance_waiters -= 1,
        }
    }

    fn can_admit(&self, class: ProxySqliteWriteClass) -> bool {
        if self.active.is_some() {
            return false;
        }
        match class {
            ProxySqliteWriteClass::P1Terminal => true,
            ProxySqliteWriteClass::InteractiveProxy => self.p1_waiters == 0,
            ProxySqliteWriteClass::P2Derived => {
                self.p1_waiters == 0 && self.interactive_waiters == 0
            }
            ProxySqliteWriteClass::MaintenanceRetention => {
                self.p1_waiters == 0 && self.interactive_waiters == 0 && self.p2_waiters == 0
            }
        }
    }

    fn maintenance_fairness_deadline(
        &self,
        requested_deadline: Instant,
        fairness_interval: Duration,
    ) -> Instant {
        self.last_maintenance_fairness_admission
            .map(|last| requested_deadline.max(last + fairness_interval))
            .unwrap_or(requested_deadline)
    }

    fn can_admit_maintenance_fairness(&self, deadline: Instant, now: Instant) -> bool {
        // Fairness prevents perpetual maintenance starvation, but it never lets an
        // already queued P1 terminal write lose to work that has not started yet.
        self.active.is_none() && self.p1_waiters == 0 && now >= deadline
    }
}

#[derive(Debug)]
pub(crate) struct ProxySqliteWriteCoordinator {
    coordinated: bool,
    state: Mutex<CoordinatorState>,
    notify: Notify,
}

impl ProxySqliteWriteCoordinator {
    fn from_env() -> Self {
        let coordinated = !matches!(
            std::env::var(PROXY_SQLITE_WRITE_COORDINATOR_MODE_ENV),
            Ok(value) if value.trim().eq_ignore_ascii_case("legacy")
        );
        Self {
            coordinated,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        }
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
        class: ProxySqliteWriteClass,
    ) -> ProxySqliteWritePermit {
        let requested_at = Instant::now();
        if !self.coordinated {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.direct_write_bypass_count = state.direct_write_bypass_count.saturating_add(1);
            return ProxySqliteWritePermit {
                coordinator: self.clone(),
                class,
                coordinated: false,
                lock_wait: requested_at.elapsed(),
                notify_background_eligibility: false,
                fairness_admission: false,
            };
        }

        {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.increment(class);
        }
        if matches!(
            class,
            ProxySqliteWriteClass::P1Terminal | ProxySqliteWriteClass::InteractiveProxy
        ) {
            // A P2 operation may have reserved admission while it is still waiting to
            // start SQLite work. Wake it so it can yield before becoming a writer.
            self.notify.notify_waiters();
        }
        let mut waiter = ProxySqliteWriteWaiter {
            coordinator: self.clone(),
            class,
            registered: true,
        };
        loop {
            let notified = self.notify.notified();
            {
                let mut state = self.state.lock().expect("proxy sqlite coordinator state");
                if state.can_admit(class) {
                    state.decrement(class);
                    waiter.registered = false;
                    state.active = Some(class);
                    return ProxySqliteWritePermit {
                        coordinator: self.clone(),
                        class,
                        coordinated: true,
                        lock_wait: requested_at.elapsed(),
                        notify_background_eligibility: true,
                        fairness_admission: false,
                    };
                }
            }
            notified.await;
        }
    }

    pub(crate) fn try_acquire(
        self: &Arc<Self>,
        class: ProxySqliteWriteClass,
    ) -> Option<ProxySqliteWritePermit> {
        let requested_at = Instant::now();
        if !self.coordinated {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.direct_write_bypass_count = state.direct_write_bypass_count.saturating_add(1);
            return Some(ProxySqliteWritePermit {
                coordinator: self.clone(),
                class,
                coordinated: false,
                lock_wait: requested_at.elapsed(),
                notify_background_eligibility: false,
                fairness_admission: false,
            });
        }
        let mut state = self.state.lock().expect("proxy sqlite coordinator state");
        if !state.can_admit(class) {
            return None;
        }
        state.active = Some(class);
        Some(ProxySqliteWritePermit {
            coordinator: self.clone(),
            class,
            coordinated: true,
            lock_wait: requested_at.elapsed(),
            notify_background_eligibility: true,
            fairness_admission: false,
        })
    }

    pub(crate) async fn acquire_maintenance(
        self: &Arc<Self>,
        fairness_interval: Duration,
    ) -> ProxySqliteWritePermit {
        let requested_at = Instant::now();
        let class = ProxySqliteWriteClass::MaintenanceRetention;
        if !self.coordinated {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.direct_write_bypass_count = state.direct_write_bypass_count.saturating_add(1);
            return ProxySqliteWritePermit {
                coordinator: self.clone(),
                class,
                coordinated: false,
                lock_wait: requested_at.elapsed(),
                notify_background_eligibility: false,
                fairness_admission: false,
            };
        }

        {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.increment(class);
        }
        let mut waiter = ProxySqliteWriteWaiter {
            coordinator: self.clone(),
            class,
            registered: true,
        };
        let fairness_deadline = requested_at + fairness_interval;
        loop {
            let now = Instant::now();
            let notified = self.notify.notified();
            let next_fairness_deadline = {
                let mut state = self.state.lock().expect("proxy sqlite coordinator state");
                let next_fairness_deadline =
                    state.maintenance_fairness_deadline(fairness_deadline, fairness_interval);
                let fairness_admission =
                    state.can_admit_maintenance_fairness(next_fairness_deadline, now);
                if state.can_admit(class) || fairness_admission {
                    state.decrement(class);
                    waiter.registered = false;
                    state.active = Some(class);
                    if fairness_admission {
                        state.maintenance_fairness_admissions =
                            state.maintenance_fairness_admissions.saturating_add(1);
                        state.last_maintenance_fairness_admission = Some(now);
                    }
                    return ProxySqliteWritePermit {
                        coordinator: self.clone(),
                        class,
                        coordinated: true,
                        lock_wait: requested_at.elapsed(),
                        notify_background_eligibility: true,
                        fairness_admission,
                    };
                }
                next_fairness_deadline
            };
            if Instant::now() >= next_fairness_deadline {
                // The fairness deadline only bypasses queued higher-priority work. An
                // active writer remains a hard boundary, so wait for its notification
                // instead of spinning a zero-delay timer until it releases.
                notified.await;
            } else {
                let until_fairness =
                    next_fairness_deadline.saturating_duration_since(Instant::now());
                tokio::select! {
                    _ = notified => {}
                    _ = tokio::time::sleep(until_fairness) => {}
                }
            }
        }
    }

    pub(crate) async fn acquire_maintenance_cancellable(
        self: &Arc<Self>,
        fairness_interval: Duration,
        cancel: &CancellationToken,
    ) -> Option<ProxySqliteWritePermit> {
        tokio::select! {
            _ = cancel.cancelled() => None,
            permit = self.acquire_maintenance(fairness_interval) => Some(permit),
        }
    }

    pub(crate) async fn snapshot(&self) -> ProxySqliteWriteCoordinatorSnapshot {
        let state = self.state.lock().expect("proxy sqlite coordinator state");
        ProxySqliteWriteCoordinatorSnapshot {
            mode: if self.coordinated {
                "coordinated"
            } else {
                "legacy"
            }
            .to_string(),
            active_write_class: state.active.map(|class| class.as_str().to_string()),
            p1_waiter_count: state.p1_waiters,
            interactive_waiter_count: state.interactive_waiters,
            p2_waiter_count: state.p2_waiters,
            maintenance_waiter_count: state.maintenance_waiters,
            maintenance_fairness_admission_count: state.maintenance_fairness_admissions,
            direct_write_bypass_count: state.direct_write_bypass_count,
        }
    }

    pub(crate) fn p2_should_yield(&self) -> bool {
        let state = self.state.lock().expect("proxy sqlite coordinator state");
        state.p1_waiters > 0 || state.interactive_waiters > 0
    }

    pub(crate) async fn wait_for_p2_preemption(&self) {
        loop {
            let notified = self.notify.notified();
            if self.p2_should_yield() {
                return;
            }
            notified.await;
        }
    }
}

pub(crate) struct ProxySqliteWritePermit {
    coordinator: Arc<ProxySqliteWriteCoordinator>,
    class: ProxySqliteWriteClass,
    coordinated: bool,
    lock_wait: Duration,
    notify_background_eligibility: bool,
    fairness_admission: bool,
}

impl ProxySqliteWritePermit {
    pub(crate) fn lock_wait(&self) -> Duration {
        self.lock_wait
    }

    pub(crate) fn write_class(&self) -> &'static str {
        self.class.as_str()
    }

    pub(crate) fn fairness_admission(&self) -> bool {
        self.fairness_admission
    }

    pub(crate) fn revoke_fairness_admission(&mut self) {
        if !self.coordinated || !self.fairness_admission {
            return;
        }
        let mut state = self
            .coordinator
            .state
            .lock()
            .expect("proxy sqlite coordinator state");
        if state.active == Some(self.class) {
            state.maintenance_fairness_admissions =
                state.maintenance_fairness_admissions.saturating_sub(1);
            state.last_maintenance_fairness_admission = None;
        }
        self.fairness_admission = false;
    }

    pub(crate) fn suppress_background_eligibility_wakeup(&mut self) {
        self.notify_background_eligibility = false;
    }
}

struct ProxySqliteWriteWaiter {
    coordinator: Arc<ProxySqliteWriteCoordinator>,
    class: ProxySqliteWriteClass,
    registered: bool,
}

impl Drop for ProxySqliteWriteWaiter {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        {
            let mut state = self
                .coordinator
                .state
                .lock()
                .expect("proxy sqlite coordinator state");
            state.decrement(self.class);
        }
        self.coordinator.notify.notify_waiters();
    }
}

impl Drop for ProxySqliteWritePermit {
    fn drop(&mut self) {
        if !self.coordinated {
            return;
        }
        {
            let mut state = self
                .coordinator
                .state
                .lock()
                .expect("proxy sqlite coordinator state");
            state.active = None;
        }
        self.coordinator.notify.notify_waiters();
        if self.notify_background_eligibility {
            crate::db_pressure::global_db_pressure_gate().notify_background_eligibility();
        }
    }
}

pub(crate) fn proxy_sqlite_write_coordinator() -> Arc<ProxySqliteWriteCoordinator> {
    static COORDINATOR: OnceLock<Arc<ProxySqliteWriteCoordinator>> = OnceLock::new();
    COORDINATOR
        .get_or_init(|| Arc::new(ProxySqliteWriteCoordinator::from_env()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_for_waiters(
        coordinator: &ProxySqliteWriteCoordinator,
        predicate: impl Fn(&ProxySqliteWriteCoordinatorSnapshot) -> bool,
    ) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = coordinator.snapshot().await;
                if predicate(&snapshot) {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("coordinator waiters must register before the test proceeds");
    }

    #[tokio::test]
    async fn p1_is_admitted_before_waiting_interactive_and_p2_work() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator
            .acquire(ProxySqliteWriteClass::InteractiveProxy)
            .await;
        let p2 = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.acquire(ProxySqliteWriteClass::P2Derived).await }
        });
        let p1 = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await }
        });
        tokio::task::yield_now().await;
        drop(active);
        let p1_permit = tokio::time::timeout(Duration::from_secs(1), p1)
            .await
            .expect("P1 admitted")
            .expect("P1 task");
        assert!(!p2.is_finished());
        drop(p1_permit);
        tokio::time::timeout(Duration::from_secs(1), p2)
            .await
            .expect("P2 admitted")
            .expect("P2 task");
    }

    #[tokio::test]
    async fn cancelled_waiter_releases_priority_registration() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator
            .acquire(ProxySqliteWriteClass::InteractiveProxy)
            .await;
        let waiter = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await }
        });
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(coordinator.snapshot().await.p1_waiter_count, 0);
        drop(active);
        let p2 = tokio::time::timeout(
            Duration::from_secs(1),
            coordinator.acquire(ProxySqliteWriteClass::P2Derived),
        )
        .await
        .expect("cancelled P1 waiter must not block P2");
        drop(p2);
    }

    #[tokio::test]
    async fn p2_try_acquire_defers_without_registering_a_waiter_or_blocking_p1() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator
            .acquire(ProxySqliteWriteClass::InteractiveProxy)
            .await;

        assert!(
            coordinator
                .try_acquire(ProxySqliteWriteClass::P2Derived)
                .is_none()
        );
        assert_eq!(coordinator.snapshot().await.p2_waiter_count, 0);

        let p1 = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await }
        });
        tokio::task::yield_now().await;
        drop(active);
        let permit = tokio::time::timeout(Duration::from_secs(1), p1)
            .await
            .expect("P1 admitted after active writer releases")
            .expect("P1 task");
        drop(permit);
    }

    #[tokio::test]
    async fn maintenance_fairness_admits_one_waiter_after_its_deadline() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator.acquire(ProxySqliteWriteClass::P2Derived).await;
        let maintenance = tokio::spawn({
            let coordinator = coordinator.clone();
            async move {
                coordinator
                    .acquire_maintenance(Duration::from_millis(20))
                    .await
            }
        });

        wait_for_waiters(&coordinator, |snapshot| {
            snapshot.maintenance_waiter_count == 1
        })
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!maintenance.is_finished());
        drop(active);

        let permit = tokio::time::timeout(Duration::from_secs(1), maintenance)
            .await
            .expect("maintenance admitted after active write releases")
            .expect("maintenance task");
        assert!(permit.fairness_admission());
        assert_eq!(
            coordinator
                .snapshot()
                .await
                .maintenance_fairness_admission_count,
            1
        );
    }

    #[tokio::test]
    async fn maintenance_fairness_never_overtakes_a_queued_p1() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator.acquire(ProxySqliteWriteClass::P2Derived).await;
        let maintenance = tokio::spawn({
            let coordinator = coordinator.clone();
            async move {
                coordinator
                    .acquire_maintenance(Duration::from_millis(20))
                    .await
            }
        });
        let p1 = tokio::spawn({
            let coordinator = coordinator.clone();
            async move { coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await }
        });

        wait_for_waiters(&coordinator, |snapshot| {
            snapshot.maintenance_waiter_count == 1 && snapshot.p1_waiter_count == 1
        })
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(active);
        let p1_permit = tokio::time::timeout(Duration::from_secs(1), p1)
            .await
            .expect("queued P1 admitted before fairness maintenance")
            .expect("P1 task");
        assert!(!maintenance.is_finished());
        drop(p1_permit);
        let maintenance_permit = tokio::time::timeout(Duration::from_secs(1), maintenance)
            .await
            .expect("maintenance admitted after queued P1")
            .expect("maintenance task");
        assert!(maintenance_permit.fairness_admission());
    }

    #[tokio::test]
    async fn revoking_a_fairness_admission_does_not_spend_the_token() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator.acquire(ProxySqliteWriteClass::P2Derived).await;
        let maintenance = tokio::spawn({
            let coordinator = coordinator.clone();
            async move {
                coordinator
                    .acquire_maintenance(Duration::from_millis(20))
                    .await
            }
        });
        wait_for_waiters(&coordinator, |snapshot| {
            snapshot.maintenance_waiter_count == 1
        })
        .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(active);
        let mut permit = tokio::time::timeout(Duration::from_secs(1), maintenance)
            .await
            .expect("fairness maintenance admitted")
            .expect("maintenance task");
        assert!(permit.fairness_admission());
        permit.revoke_fairness_admission();
        assert_eq!(
            coordinator
                .snapshot()
                .await
                .maintenance_fairness_admission_count,
            0
        );
    }

    #[tokio::test]
    async fn retention_write_scheduler_lock_and_cancel_releases_maintenance_waiter() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await;
        let maintenance = tokio::spawn({
            let coordinator = coordinator.clone();
            async move {
                coordinator
                    .acquire_maintenance(Duration::from_secs(60))
                    .await
            }
        });
        wait_for_waiters(&coordinator, |snapshot| {
            snapshot.maintenance_waiter_count == 1
        })
        .await;
        assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 1);
        maintenance.abort();
        let _ = maintenance.await;
        assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
        drop(active);
    }

    #[tokio::test]
    async fn cancellable_maintenance_admission_releases_waiter_immediately() {
        let coordinator = Arc::new(ProxySqliteWriteCoordinator {
            coordinated: true,
            state: Mutex::new(CoordinatorState::default()),
            notify: Notify::new(),
        });
        let active = coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await;
        let cancel = CancellationToken::new();
        let waiter = tokio::spawn({
            let coordinator = coordinator.clone();
            let cancel = cancel.clone();
            async move {
                coordinator
                    .acquire_maintenance_cancellable(Duration::from_secs(60), &cancel)
                    .await
            }
        });
        wait_for_waiters(&coordinator, |snapshot| {
            snapshot.maintenance_waiter_count == 1
        })
        .await;
        assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 1);
        cancel.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), waiter)
                .await
                .expect("cancelled maintenance waiter completes")
                .expect("maintenance task")
                .is_none()
        );
        assert_eq!(coordinator.snapshot().await.maintenance_waiter_count, 0);
        drop(active);
    }

    #[test]
    fn retention_write_scheduler_fairness_rate_limits_the_next_token() {
        let now = Instant::now();
        let interval = Duration::from_secs(15);
        let mut state = CoordinatorState::default();
        assert_eq!(state.maintenance_fairness_deadline(now, interval), now);

        state.last_maintenance_fairness_admission = Some(now);
        assert_eq!(
            state.maintenance_fairness_deadline(now, interval),
            now + interval
        );
    }
}
