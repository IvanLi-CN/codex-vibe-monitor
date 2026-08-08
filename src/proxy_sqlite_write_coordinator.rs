use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::Notify;

pub(crate) const PROXY_SQLITE_WRITE_COORDINATOR_MODE_ENV: &str =
    "PROXY_SQLITE_WRITE_COORDINATOR_MODE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxySqliteWriteClass {
    P1Terminal,
    InteractiveProxy,
    P2Derived,
}

impl ProxySqliteWriteClass {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::P1Terminal => "p1_terminal",
            Self::InteractiveProxy => "interactive_proxy",
            Self::P2Derived => "p2_derived",
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
    pub(crate) direct_write_bypass_count: u64,
}

#[derive(Debug, Default)]
struct CoordinatorState {
    active: Option<ProxySqliteWriteClass>,
    p1_waiters: usize,
    interactive_waiters: usize,
    p2_waiters: usize,
    direct_write_bypass_count: u64,
}

impl CoordinatorState {
    fn increment(&mut self, class: ProxySqliteWriteClass) {
        match class {
            ProxySqliteWriteClass::P1Terminal => self.p1_waiters += 1,
            ProxySqliteWriteClass::InteractiveProxy => self.interactive_waiters += 1,
            ProxySqliteWriteClass::P2Derived => self.p2_waiters += 1,
        }
    }

    fn decrement(&mut self, class: ProxySqliteWriteClass) {
        match class {
            ProxySqliteWriteClass::P1Terminal => self.p1_waiters -= 1,
            ProxySqliteWriteClass::InteractiveProxy => self.interactive_waiters -= 1,
            ProxySqliteWriteClass::P2Derived => self.p2_waiters -= 1,
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
        }
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
        if !self.coordinated {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.direct_write_bypass_count = state.direct_write_bypass_count.saturating_add(1);
            return ProxySqliteWritePermit {
                coordinator: self.clone(),
                class,
                coordinated: false,
                admitted_at: Instant::now(),
            };
        }

        {
            let mut state = self.state.lock().expect("proxy sqlite coordinator state");
            state.increment(class);
        }
        loop {
            let notified = self.notify.notified();
            {
                let mut state = self.state.lock().expect("proxy sqlite coordinator state");
                if state.can_admit(class) {
                    state.decrement(class);
                    state.active = Some(class);
                    return ProxySqliteWritePermit {
                        coordinator: self.clone(),
                        class,
                        coordinated: true,
                        admitted_at: Instant::now(),
                    };
                }
            }
            notified.await;
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
            direct_write_bypass_count: state.direct_write_bypass_count,
        }
    }
}

pub(crate) struct ProxySqliteWritePermit {
    coordinator: Arc<ProxySqliteWriteCoordinator>,
    class: ProxySqliteWriteClass,
    coordinated: bool,
    admitted_at: Instant,
}

impl ProxySqliteWritePermit {
    pub(crate) fn lock_wait(&self) -> Duration {
        self.admitted_at.elapsed()
    }

    pub(crate) fn write_class(&self) -> &'static str {
        self.class.as_str()
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
}
