use crate::{Result, Utc, anyhow, format_utc_iso_millis};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Instant,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentTaskExecution {
    pub(crate) execution_id: u64,
    pub(crate) execution_uid: String,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) active_child_task_key: Option<String>,
    pub(crate) active_child_title: Option<String>,
    pub(crate) trigger_kind: String,
    pub(crate) phase: String,
    pub(crate) execution_class: Option<String>,
    pub(crate) started_at: String,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskRuntimeSnapshot {
    pub(crate) observed_at: String,
    pub(crate) active_runs: Vec<CurrentTaskExecution>,
    pub(crate) queued_runs: Vec<crate::maintenance_store::QueuedTaskRun>,
    pub(crate) queued_runs_available: bool,
    pub(crate) admission_waits: Vec<crate::maintenance_store::TaskAdmissionWait>,
    pub(crate) admission_waits_available: bool,
}

#[derive(Debug)]
struct ActiveExecution {
    execution_id: u64,
    execution_uid: String,
    task_key: String,
    title: String,
    active_child_task_key: Option<String>,
    active_child_title: Option<String>,
    trigger_kind: String,
    phase: String,
    execution_class: Option<String>,
    started_at: String,
    started_clock: Instant,
}

#[derive(Debug, Default)]
struct Registry {
    active: BTreeMap<u64, ActiveExecution>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static NEXT_EXECUTION_ID: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

fn managed_observations() -> &'static Mutex<HashMap<i64, Weak<ObservationLease>>> {
    static OBSERVATIONS: OnceLock<Mutex<HashMap<i64, Weak<ObservationLease>>>> = OnceLock::new();
    OBSERVATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, Clone)]
pub(crate) struct TaskExecutionObservation {
    inner: Arc<ObservationLease>,
}

#[derive(Debug)]
struct ObservationLease {
    execution_id: u64,
    execution_uid: String,
    managed_run_id: Option<i64>,
    started_clock: Instant,
    ended: AtomicBool,
}

impl Drop for ObservationLease {
    fn drop(&mut self) {
        if !self.ended.swap(true, Ordering::AcqRel) {
            if let Ok(mut registry) = registry().lock() {
                registry.active.remove(&self.execution_id);
            }
            crate::task_timeline::execution_unknown(
                self.execution_uid.clone(),
                format_utc_iso_millis(Utc::now()),
            );
        }
        if let Some(managed_run_id) = self.managed_run_id
            && let Ok(mut observations) = managed_observations().lock()
            && observations
                .get(&managed_run_id)
                .is_some_and(|observation| std::ptr::eq(observation.as_ptr(), self))
        {
            observations.remove(&managed_run_id);
        }
    }
}

impl TaskExecutionObservation {
    pub(crate) fn begin(
        task_key: &str,
        title: &str,
        trigger_kind: &str,
        execution_class: Option<&str>,
        phase: &str,
    ) -> Self {
        Self::begin_for_managed_run(task_key, title, trigger_kind, execution_class, phase, None)
    }

    pub(crate) fn begin_for_managed_run(
        task_key: &str,
        title: &str,
        trigger_kind: &str,
        execution_class: Option<&str>,
        phase: &str,
        managed_run_id: Option<i64>,
    ) -> Self {
        let execution_id = NEXT_EXECUTION_ID.fetch_add(1, Ordering::Relaxed);
        let execution_uid = nanoid::nanoid!();
        let started_at = format_utc_iso_millis(Utc::now());
        let started_clock = Instant::now();
        let execution = ActiveExecution {
            execution_id,
            execution_uid: execution_uid.clone(),
            task_key: task_key.to_string(),
            title: title.to_string(),
            active_child_task_key: None,
            active_child_title: None,
            trigger_kind: trigger_kind.to_string(),
            phase: phase.to_string(),
            execution_class: execution_class.map(str::to_string),
            started_at: started_at.clone(),
            started_clock,
        };
        if let Ok(mut registry) = registry().lock() {
            registry.active.insert(execution_id, execution);
        }
        crate::task_timeline::execution_started(
            execution_uid.clone(),
            task_key.to_string(),
            title.to_string(),
            trigger_kind.to_string(),
            execution_class.map(str::to_string),
            started_at,
            managed_run_id,
        );
        let observation = Self {
            inner: Arc::new(ObservationLease {
                execution_id,
                execution_uid,
                managed_run_id,
                started_clock,
                ended: AtomicBool::new(false),
            }),
        };
        if let Some(managed_run_id) = managed_run_id
            && let Ok(mut observations) = managed_observations().lock()
        {
            observations.insert(managed_run_id, Arc::downgrade(&observation.inner));
        }
        observation
    }

    pub(crate) fn for_managed_run(managed_run_id: i64) -> Option<Self> {
        managed_observations().lock().ok().and_then(|observations| {
            observations
                .get(&managed_run_id)
                .and_then(Weak::upgrade)
                .map(|inner| Self { inner })
        })
    }

    pub(crate) fn set_phase(&self, phase: &str) {
        if let Ok(mut registry) = registry().lock()
            && let Some(execution) = registry.active.get_mut(&self.inner.execution_id)
        {
            execution.phase = phase.to_string();
        }
    }

    pub(crate) fn set_child(&self, task_key: &str, title: &str) {
        if let Ok(mut registry) = registry().lock()
            && let Some(execution) = registry.active.get_mut(&self.inner.execution_id)
        {
            execution.active_child_task_key = Some(task_key.to_string());
            execution.active_child_title = Some(title.to_string());
        }
        crate::task_timeline::execution_child_changed(
            self.inner.execution_uid.clone(),
            Some(task_key.to_string()),
            Some(title.to_string()),
        );
    }

    pub(crate) fn clear_child(&self) {
        if let Ok(mut registry) = registry().lock()
            && let Some(execution) = registry.active.get_mut(&self.inner.execution_id)
        {
            execution.active_child_task_key = None;
            execution.active_child_title = None;
        }
        crate::task_timeline::execution_child_changed(self.inner.execution_uid.clone(), None, None);
    }

    pub(crate) fn finish(&self) {
        self.finish_with_status("unknown");
    }

    pub(crate) fn finish_with_status(&self, status: &str) {
        if self.inner.ended.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut registry) = registry().lock() {
            registry.active.remove(&self.inner.execution_id);
        }
        if let Ok(mut observations) = managed_observations().lock() {
            observations.retain(|_, observation| {
                observation
                    .upgrade()
                    .is_some_and(|lease| lease.execution_id != self.inner.execution_id)
            });
        }
        crate::task_timeline::execution_finished(
            self.inner.execution_uid.clone(),
            format_utc_iso_millis(Utc::now()),
            self.inner
                .started_clock
                .elapsed()
                .as_millis()
                .min(u64::MAX as u128) as u64,
            status,
        );
    }
}

pub(crate) fn task_runtime_snapshot() -> Result<TaskRuntimeSnapshot> {
    let observed_at = format_utc_iso_millis(Utc::now());
    let active_runs = registry()
        .lock()
        .map_err(|_| anyhow!("task runtime observation registry is unavailable"))
        .map(|registry| {
            registry
                .active
                .values()
                .map(|execution| CurrentTaskExecution {
                    execution_id: execution.execution_id,
                    execution_uid: execution.execution_uid.clone(),
                    task_key: execution.task_key.clone(),
                    title: execution.title.clone(),
                    active_child_task_key: execution.active_child_task_key.clone(),
                    active_child_title: execution.active_child_title.clone(),
                    trigger_kind: execution.trigger_kind.clone(),
                    phase: execution.phase.clone(),
                    execution_class: execution.execution_class.clone(),
                    started_at: execution.started_at.clone(),
                    elapsed_ms: execution
                        .started_clock
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128) as u64,
                })
                .collect()
        })?;
    Ok(TaskRuntimeSnapshot {
        observed_at,
        active_runs,
        queued_runs: Vec::new(),
        queued_runs_available: false,
        admission_waits: Vec::new(),
        admission_waits_available: false,
    })
}

#[cfg(test)]
pub(crate) fn clear_task_runtime_observation_for_tests() {
    if let Ok(mut registry) = registry().lock() {
        registry.active.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TaskExecutionObservation, clear_task_runtime_observation_for_tests, task_runtime_snapshot,
    };
    use std::{
        sync::{Mutex, OnceLock},
        thread,
        time::Duration,
    };

    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock runtime observation test")
    }

    #[test]
    fn scope_reports_monotonic_elapsed_and_removes_on_drop() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let observation = TaskExecutionObservation::begin(
            "dashboard_runtime_projection_reconcile",
            "仪表盘运行投影校对",
            "interval",
            Some("p2_derived"),
            "processing",
        );
        observation.set_child("dashboard_runtime_projection_reconcile", "当前投影");
        let first = task_runtime_snapshot().expect("runtime snapshot should be readable");
        assert_eq!(first.active_runs.len(), 1);
        assert_eq!(
            first.active_runs[0].active_child_title.as_deref(),
            Some("当前投影")
        );
        thread::sleep(Duration::from_millis(2));
        let second = task_runtime_snapshot().expect("runtime snapshot should be readable");
        assert!(second.active_runs[0].elapsed_ms >= first.active_runs[0].elapsed_ms);
        drop(observation);
        assert!(
            task_runtime_snapshot()
                .expect("runtime snapshot should be readable")
                .active_runs
                .is_empty()
        );
    }

    #[test]
    fn cloned_observation_keeps_one_execution_identity() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let observation = TaskExecutionObservation::begin(
            "retention_archive",
            "数据保留与归档",
            "startup",
            None,
            "processing",
        );
        let clone = observation.clone();
        let snapshot = task_runtime_snapshot().expect("runtime snapshot should be readable");
        assert_eq!(snapshot.active_runs.len(), 1);
        assert_eq!(snapshot.active_runs[0].task_key, "retention_archive");
        drop(observation);
        assert_eq!(
            task_runtime_snapshot()
                .expect("runtime snapshot should be readable")
                .active_runs
                .len(),
            1
        );
        drop(clone);
        assert!(
            task_runtime_snapshot()
                .expect("runtime snapshot should be readable")
                .active_runs
                .is_empty()
        );
    }

    #[test]
    fn managed_children_reuse_the_parent_execution_identity() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let managed_run_id = 9_223_372_036_854_700_000;
        let parent = TaskExecutionObservation::begin_for_managed_run(
            "startup_backfill",
            "启动回填",
            "manual",
            Some("p2_derived"),
            "processing",
            Some(managed_run_id),
        );
        let child = TaskExecutionObservation::for_managed_run(managed_run_id)
            .expect("resolve the managed parent observation");
        assert_eq!(child.inner.execution_id, parent.inner.execution_id);
        child.set_child("startup_backfill.proxy_usage", "代理用量回填");
        let snapshot = task_runtime_snapshot().expect("read shared runtime observation");
        assert_eq!(snapshot.active_runs.len(), 1);
        assert_eq!(
            snapshot.active_runs[0].active_child_task_key.as_deref(),
            Some("startup_backfill.proxy_usage")
        );
        drop(child);
        parent.finish_with_status("success");
        assert!(TaskExecutionObservation::for_managed_run(managed_run_id).is_none());
        assert!(
            task_runtime_snapshot()
                .expect("read finished runtime observation")
                .active_runs
                .is_empty()
        );
    }
}
