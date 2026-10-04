use crate::{Result, Utc, anyhow, format_utc_iso_millis};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
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

fn workload_observations()
-> &'static Mutex<HashMap<String, crate::maintenance_store::TaskWorkloadSample>> {
    static OBSERVATIONS: OnceLock<
        Mutex<HashMap<String, crate::maintenance_store::TaskWorkloadSample>>,
    > = OnceLock::new();
    OBSERVATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn last_workload_notifications() -> &'static Mutex<HashMap<String, Instant>> {
    static NOTIFICATIONS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    NOTIFICATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

tokio::task_local! {
    static ACTIVE_MANAGED_TASK_OBSERVATION: TaskExecutionObservation;
}

pub(crate) fn workload_sample(
    task_key: &str,
) -> Option<crate::maintenance_store::TaskWorkloadSample> {
    workload_observations().lock().ok().and_then(|samples| {
        samples
            .values()
            .filter(|sample| sample.task_key == task_key)
            .max_by(|left, right| {
                left.attempted_at
                    .cmp(&right.attempted_at)
                    .then_with(|| left.execution_uid.cmp(&right.execution_uid))
            })
            .cloned()
    })
}

fn update_workload_sample(
    execution_uid: &str,
    task_key: &str,
    force_notify: bool,
    update: impl FnOnce(&mut crate::maintenance_store::TaskWorkloadSample),
) {
    let sample = {
        let Ok(mut samples) = workload_observations().lock() else {
            return;
        };
        let Some(sample) = samples.get_mut(&format!("{execution_uid}:{task_key}")) else {
            return;
        };
        update(sample);
        sample.sequence = sample.sequence.saturating_add(1);
        sample.clone()
    };
    let notification_key = format!("{execution_uid}:{task_key}");
    let should_notify = if force_notify {
        if let Ok(mut notifications) = last_workload_notifications().lock() {
            notifications.remove(&notification_key);
        }
        true
    } else if let Ok(mut notifications) = last_workload_notifications().lock() {
        let now = Instant::now();
        let notify = notifications
            .get(&notification_key)
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(1));
        if notify {
            notifications.insert(notification_key, now);
        }
        notify
    } else {
        false
    };
    if should_notify {
        crate::task_timeline::workload_sample_changed(sample);
    }
}

fn sample_metric<'a>(
    sample: &'a mut crate::maintenance_store::TaskWorkloadSample,
    key: &str,
) -> Option<&'a mut crate::maintenance_store::TaskWorkloadMetric> {
    match key {
        "pending" => sample.pending.as_mut(),
        "discovered" => sample.discovered.as_mut(),
        "processed" => sample.processed.as_mut(),
        _ => None,
    }
}

fn new_workload_sample(
    execution_uid: &str,
    task_key: &str,
    trigger_kind: &str,
    managed_run_id: Option<i64>,
    started_at: String,
) -> crate::maintenance_store::TaskWorkloadSample {
    let capabilities = crate::maintenance_store::task_measurement_capabilities(task_key);
    let mut sample = crate::maintenance_store::TaskWorkloadSample {
        sample_id: format!("{execution_uid}:{task_key}"),
        execution_uid: execution_uid.to_string(),
        managed_run_id,
        task_key: task_key.to_string(),
        trigger_kind: trigger_kind.to_string(),
        attempted_at: started_at.clone(),
        actual_started_at: Some(started_at.clone()),
        finished_at: None,
        status: "running".to_string(),
        reason: None,
        sequence: 0,
        pending: None,
        discovered: None,
        processed: None,
        subset_relation: "unknown".to_string(),
    };
    for (capability, slot) in [
        (capabilities.pending, &mut sample.pending),
        (capabilities.discovered, &mut sample.discovered),
        (capabilities.processed, &mut sample.processed),
    ] {
        if capability.supported {
            *slot = Some(crate::maintenance_store::TaskWorkloadMetric {
                value: None,
                unit: capability.unit.unwrap_or_default(),
                scope: capability.scope.unwrap_or_default(),
                range: "not-observed".to_string(),
                observed_at: None,
                coverage: "unknown".to_string(),
            });
        }
    }
    sample
}

#[derive(Debug, Clone)]
pub(crate) struct TaskExecutionObservation {
    inner: Arc<ObservationLease>,
}

pub(crate) async fn with_managed_task_observation<F: Future>(
    observation: TaskExecutionObservation,
    future: F,
) -> F::Output {
    ACTIVE_MANAGED_TASK_OBSERVATION
        .scope(observation, future)
        .await
}

pub(crate) fn record_managed_task_processed_work(task_keys: &[&str], count: i64) {
    let _ = ACTIVE_MANAGED_TASK_OBSERVATION.try_with(|observation| {
        if task_keys.contains(&observation.inner.task_key.as_str()) {
            observation.add_processed_work(count);
        }
    });
}

#[derive(Debug)]
struct ObservationLease {
    execution_id: u64,
    execution_uid: String,
    managed_run_id: Option<i64>,
    task_key: String,
    trigger_kind: String,
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
            update_workload_sample(&self.execution_uid, &self.task_key, true, |sample| {
                sample.status = "unknown".to_string();
                sample.reason =
                    Some("execution ended without a confirmed terminal state".to_string());
                sample.finished_at = None;
            });
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
    pub(crate) fn begin_for_system_task_run(
        task_key: &str,
        trigger_kind: &str,
        phase: &str,
        managed_run_id: Option<i64>,
    ) -> Self {
        Self::begin_for_managed_run(
            task_key,
            &crate::maintenance_store::task_title_for_observation(task_key),
            trigger_kind,
            crate::maintenance_store::task_execution_class(task_key),
            phase,
            managed_run_id,
        )
    }

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
            started_at.clone(),
            managed_run_id,
        );
        let sample = new_workload_sample(
            &execution_uid,
            task_key,
            trigger_kind,
            managed_run_id,
            started_at.clone(),
        );
        if let Ok(mut samples) = workload_observations().lock() {
            samples.retain(|_, existing| {
                existing.task_key != task_key || existing.status == "running"
            });
            samples.insert(sample.sample_id.clone(), sample.clone());
        }
        crate::task_timeline::workload_sample_changed(sample);
        if let Ok(mut notifications) = last_workload_notifications().lock() {
            notifications.insert(format!("{execution_uid}:{task_key}"), Instant::now());
        }
        let observation = Self {
            inner: Arc::new(ObservationLease {
                execution_id,
                execution_uid,
                managed_run_id,
                task_key: task_key.to_string(),
                trigger_kind: trigger_kind.to_string(),
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

    pub(crate) fn begin_subtask_workload(&self, task_key: &str) -> TaskWorkloadObservation {
        if self.inner.task_key != task_key {
            let sample = new_workload_sample(
                &self.inner.execution_uid,
                task_key,
                &self.inner.trigger_kind,
                self.inner.managed_run_id,
                format_utc_iso_millis(Utc::now()),
            );
            if let Ok(mut samples) = workload_observations().lock() {
                samples.insert(sample.sample_id.clone(), sample.clone());
            }
            crate::task_timeline::workload_sample_changed(sample);
            if let Ok(mut notifications) = last_workload_notifications().lock() {
                notifications.insert(
                    format!("{}:{task_key}", self.inner.execution_uid),
                    Instant::now(),
                );
            }
        }
        TaskWorkloadObservation {
            execution_uid: self.inner.execution_uid.clone(),
            task_key: task_key.to_string(),
            processed_count: None,
            finalized: false,
        }
    }

    pub(crate) fn set_phase(&self, phase: &str) {
        if let Ok(mut registry) = registry().lock()
            && let Some(execution) = registry.active.get_mut(&self.inner.execution_id)
        {
            execution.phase = phase.to_string();
        }
        crate::task_timeline::notify_runtime_changed();
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

    pub(crate) fn set_pending_population(
        &self,
        value: Option<i64>,
        observed_at: String,
        scope: String,
        range: String,
        coverage: &str,
    ) {
        let execution_uid = &self.inner.execution_uid;
        let task_key = &self.inner.task_key;
        update_workload_sample(execution_uid, task_key, false, |sample| {
            if let Some(metric) = sample_metric(sample, "pending") {
                metric.value = value;
                metric.observed_at = Some(observed_at);
                metric.scope = scope;
                metric.range = range;
                metric.coverage = coverage.to_string();
            }
            let pending_scope = sample
                .pending
                .as_ref()
                .map(|metric| metric.scope.clone())
                .unwrap_or_default();
            let pending_range = sample
                .pending
                .as_ref()
                .map(|metric| metric.range.clone())
                .unwrap_or_default();
            for key in ["discovered", "processed"] {
                if let Some(metric) = sample_metric(sample, key) {
                    metric.scope = pending_scope.clone();
                    metric.range = pending_range.clone();
                }
            }
            refresh_subset_relation(sample);
        });
    }

    pub(crate) fn set_discovered_work(&self, value: i64, observed_at: String, range: String) {
        let execution_uid = &self.inner.execution_uid;
        let task_key = &self.inner.task_key;
        update_workload_sample(execution_uid, task_key, false, |sample| {
            for key in ["discovered", "processed"] {
                if let Some(metric) = sample_metric(sample, key) {
                    metric.range = range.clone();
                }
            }
            if let Some(metric) = sample_metric(sample, "discovered") {
                metric.value = Some(value.max(0));
                metric.observed_at = Some(observed_at);
                metric.coverage = "window".to_string();
            }
            refresh_subset_relation(sample);
        });
    }

    pub(crate) fn set_processed_work(&self, count: i64) {
        let processed_count = count.max(0);
        update_workload_sample(
            &self.inner.execution_uid,
            &self.inner.task_key,
            false,
            |sample| {
                if let Some(metric) = sample_metric(sample, "processed") {
                    metric.value = Some(processed_count);
                    metric.range = "run-window".to_string();
                    metric.observed_at = Some(format_utc_iso_millis(Utc::now()));
                    metric.coverage = "window".to_string();
                }
            },
        );
    }

    pub(crate) fn add_processed_work(&self, count: i64) {
        self.add_work("processed", count);
    }

    fn add_work(&self, key: &str, count: i64) {
        if count <= 0 {
            return;
        }
        let execution_uid = &self.inner.execution_uid;
        let task_key = &self.inner.task_key;
        update_workload_sample(execution_uid, task_key, false, |sample| {
            if let Some(metric) = sample_metric(sample, key) {
                metric.value = Some(metric.value.unwrap_or(0).saturating_add(count));
                metric.observed_at = Some(format_utc_iso_millis(Utc::now()));
                metric.coverage = "window".to_string();
            }
            refresh_subset_relation(sample);
        });
    }

    pub(crate) fn finish(&self) {
        self.finish_with_status("unknown");
    }

    pub(crate) fn finish_with_status(&self, status: &str) {
        self.finish_with_status_and_reason(status, None);
    }

    pub(crate) fn finish_from_result<E: std::fmt::Display>(&self, result: &Result<(), E>) {
        match result {
            Ok(()) => self.finish_with_status("success"),
            Err(error) => {
                let reason = error.to_string();
                self.finish_with_status_and_reason("failed", Some(&reason));
            }
        }
    }

    pub(crate) fn finish_with_status_and_reason(&self, status: &str, reason: Option<&str>) {
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
        update_workload_sample(
            &self.inner.execution_uid,
            &self.inner.task_key,
            true,
            |sample| {
                sample.finished_at = Some(format_utc_iso_millis(Utc::now()));
                sample.status = status.to_string();
                let has_metric_observation =
                    [&sample.pending, &sample.discovered, &sample.processed]
                        .into_iter()
                        .flatten()
                        .any(|metric| metric.value.is_some());
                if status == "skipped" && !has_metric_observation {
                    sample.actual_started_at = None;
                }
                if let Some(reason) = reason.filter(|reason| !reason.trim().is_empty()) {
                    sample.reason = Some(reason.to_string());
                } else if status == "unknown" {
                    sample.reason = Some("execution end is unknown".to_string());
                } else if status == "skipped" {
                    sample.reason = Some("confirmed skip before measured work".to_string());
                }
                refresh_subset_relation(sample);
            },
        );
    }
}

pub(crate) struct TaskWorkloadObservation {
    execution_uid: String,
    task_key: String,
    processed_count: Option<i64>,
    finalized: bool,
}

impl TaskWorkloadObservation {
    pub(crate) fn set_processed_work(&mut self, count: i64) {
        self.processed_count = Some(count.max(0));
        let processed_count = self.processed_count;
        update_workload_sample(&self.execution_uid, &self.task_key, false, |sample| {
            if let Some(metric) = sample_metric(sample, "processed") {
                metric.value = processed_count;
                metric.range = "run-window".to_string();
                metric.observed_at = Some(format_utc_iso_millis(Utc::now()));
                metric.coverage = "window".to_string();
            }
        });
    }

    pub(crate) fn finish_with_status(&mut self, status: &str) {
        self.finish_with_status_and_reason(status, None);
    }

    pub(crate) fn finish_with_status_and_reason(&mut self, status: &str, reason: Option<&str>) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let processed_count = self.processed_count;
        update_workload_sample(&self.execution_uid, &self.task_key, true, |sample| {
            sample.finished_at = Some(format_utc_iso_millis(Utc::now()));
            sample.status = status.to_string();
            if let Some(metric) = sample_metric(sample, "processed") {
                metric.value = processed_count;
                if processed_count.is_some() {
                    metric.range = "run-window".to_string();
                }
                metric.observed_at = Some(format_utc_iso_millis(Utc::now()));
                metric.coverage = if processed_count.is_some() {
                    "window".to_string()
                } else {
                    "unknown".to_string()
                };
            }
            let has_metric_observation = [&sample.pending, &sample.discovered, &sample.processed]
                .into_iter()
                .flatten()
                .any(|metric| metric.value.is_some());
            if status == "skipped" && !has_metric_observation {
                sample.actual_started_at = None;
            }
            if let Some(reason) = reason.filter(|reason| !reason.trim().is_empty()) {
                sample.reason = Some(reason.to_string());
            } else if status == "skipped" {
                sample.reason = Some("confirmed skip before measured work".to_string());
            } else if processed_count.is_none() {
                sample.reason = Some("committed work count is incomplete".to_string());
            }
        });
    }
}

impl Drop for TaskWorkloadObservation {
    fn drop(&mut self) {
        self.finish_with_status("unknown");
    }
}

fn refresh_subset_relation(sample: &mut crate::maintenance_store::TaskWorkloadSample) {
    let metrics = (&sample.pending, &sample.discovered, &sample.processed);
    let (Some(pending), Some(discovered), Some(processed)) = metrics else {
        sample.subset_relation = "unknown".to_string();
        return;
    };
    let (Some(p), Some(d), Some(c)) = (pending.value, discovered.value, processed.value) else {
        sample.subset_relation = "unknown".to_string();
        return;
    };
    sample.subset_relation = if pending.scope == discovered.scope
        && discovered.scope == processed.scope
        && pending.range == discovered.range
        && discovered.range == processed.range
        && pending.unit == discovered.unit
        && discovered.unit == processed.unit
        && pending.coverage == "exact"
        && discovered.coverage == "window"
        && processed.coverage == "window"
        && c <= d
        && d <= p
    {
        "confirmed".to_string()
    } else {
        "incompatible".to_string()
    };
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
    if let Ok(mut observations) = managed_observations().lock() {
        observations.clear();
    }
    if let Ok(mut samples) = workload_observations().lock() {
        samples.clear();
    }
    if let Ok(mut notifications) = last_workload_notifications().lock() {
        notifications.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TaskExecutionObservation, clear_task_runtime_observation_for_tests, new_workload_sample,
        record_managed_task_processed_work, refresh_subset_relation, task_runtime_snapshot,
        with_managed_task_observation, workload_observations, workload_sample,
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
        assert_eq!(
            workload_sample("startup_backfill")
                .expect("record managed workload sample")
                .managed_run_id,
            Some(managed_run_id)
        );
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

    #[test]
    fn system_task_observation_keeps_the_managed_run_identity() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let run_id = 42;
        let observation = TaskExecutionObservation::begin_for_system_task_run(
            "forward_proxy_subscription_refresh",
            "manual",
            "processing",
            Some(run_id),
        );

        let sample = workload_sample("forward_proxy_subscription_refresh")
            .expect("record the manual workload attempt");
        assert_eq!(sample.managed_run_id, Some(run_id));
        assert_eq!(sample.trigger_kind, "manual");
        observation.finish_with_status("success");
    }

    #[test]
    fn latest_in_memory_workload_sample_breaks_timestamp_ties_by_execution_uid() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let attempted_at = "2026-10-04T12:00:00.000Z".to_string();
        let first = new_workload_sample(
            "execution-a",
            "retention_archive",
            "interval",
            None,
            attempted_at.clone(),
        );
        let second = new_workload_sample(
            "execution-b",
            "retention_archive",
            "interval",
            None,
            attempted_at,
        );
        {
            let mut samples = workload_observations()
                .lock()
                .expect("lock workload observations");
            samples.insert(first.sample_id.clone(), first);
            samples.insert(second.sample_id.clone(), second);
        }

        let latest = workload_sample("retention_archive").expect("read latest workload sample");

        assert_eq!(latest.execution_uid, "execution-b");
    }

    #[test]
    fn failed_system_task_observation_keeps_the_result_reason() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let observation = TaskExecutionObservation::begin_for_system_task_run(
            "forward_proxy_subscription_refresh",
            "interval",
            "processing",
            Some(43),
        );
        let result: Result<(), &str> = Err("subscription refresh failed");

        observation.finish_from_result(&result);

        let sample = workload_sample("forward_proxy_subscription_refresh")
            .expect("record the failed refresh attempt");
        assert_eq!(sample.status, "failed");
        assert_eq!(
            sample.reason.as_deref(),
            Some("subscription refresh failed")
        );
    }

    #[test]
    fn terminal_workload_samples_release_notification_throttle_entries() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let parent = TaskExecutionObservation::begin(
            "startup_backfill",
            "启动回填",
            "manual",
            Some("p2_derived"),
            "processing",
        );
        let parent_key = format!("{}:startup_backfill", parent.inner.execution_uid);
        let child_key = format!(
            "{}:startup_backfill.proxy_usage",
            parent.inner.execution_uid
        );
        let mut child = parent.begin_subtask_workload("startup_backfill.proxy_usage");

        {
            let notifications = super::last_workload_notifications()
                .lock()
                .expect("read workload notification throttles");
            assert!(notifications.contains_key(&parent_key));
            assert!(notifications.contains_key(&child_key));
        }

        child.finish_with_status("success");
        assert!(
            !super::last_workload_notifications()
                .lock()
                .expect("read workload notification throttles")
                .contains_key(&child_key)
        );

        parent.finish_with_status("success");
        assert!(
            !super::last_workload_notifications()
                .lock()
                .expect("read workload notification throttles")
                .contains_key(&parent_key)
        );
    }

    #[test]
    fn direct_managed_backfill_does_not_duplicate_its_workload_sample_as_a_child() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let task_key = "startup_backfill.proxy_usage";
        let observation = TaskExecutionObservation::begin_for_managed_run(
            task_key,
            "代理用量回填",
            "manual",
            Some("p2_derived"),
            "processing",
            Some(44),
        );
        let sample_before = super::workload_sample(task_key).expect("root sample exists");
        let mut child = observation.begin_subtask_workload(task_key);
        let sample_after = super::workload_sample(task_key).expect("shared sample exists");
        assert_eq!(sample_before.sample_id, sample_after.sample_id);
        assert_eq!(sample_before.execution_uid, sample_after.execution_uid);

        child.set_processed_work(12);
        child.finish_with_status("success");
        let sample = super::workload_sample(task_key).expect("completed sample exists");
        assert_eq!(
            sample.processed.as_ref().and_then(|metric| metric.value),
            Some(12)
        );
        assert_eq!(
            sample
                .processed
                .as_ref()
                .map(|metric| metric.range.as_str()),
            Some("run-window")
        );
        assert_eq!(sample.status, "success");
        observation.finish_with_status("success");
    }

    #[test]
    fn confirmed_skip_without_metrics_has_no_actual_start_and_keeps_its_reason() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let observation = TaskExecutionObservation::begin(
            "startup_backfill.account_activity_v2_coverage",
            "账号活动覆盖修复",
            "interval",
            Some("p2_derived"),
            "processing",
        );
        observation.finish_with_status_and_reason("skipped", Some("background_busy"));

        let sample = super::workload_sample("startup_backfill.account_activity_v2_coverage")
            .expect("skip sample exists");
        assert_eq!(sample.status, "skipped");
        assert!(sample.actual_started_at.is_none());
        assert_eq!(sample.reason.as_deref(), Some("background_busy"));
        assert!(sample.finished_at.is_some());
    }

    #[test]
    fn subset_relation_requires_compatible_units_and_nested_counts() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let observation = TaskExecutionObservation::begin(
            "retention_archive",
            "数据保留与归档",
            "manual",
            Some("p2_derived"),
            "processing",
        );
        let mut sample = super::workload_sample("retention_archive").expect("sample exists");
        let metric = |value: i64, coverage: &str| crate::maintenance_store::TaskWorkloadMetric {
            value: Some(value),
            unit: "invocation rows".to_string(),
            scope: "retention policy".to_string(),
            range: "id <= 1000".to_string(),
            observed_at: Some("2026-10-03T00:00:00.000Z".to_string()),
            coverage: coverage.to_string(),
        };
        sample.pending = Some(metric(1_000, "exact"));
        sample.discovered = Some(metric(100, "window"));
        sample.processed = Some(metric(60, "window"));
        refresh_subset_relation(&mut sample);
        assert_eq!(sample.subset_relation, "confirmed");

        sample.discovered.as_mut().unwrap().unit = "files".to_string();
        refresh_subset_relation(&mut sample);
        assert_eq!(sample.subset_relation, "incompatible");

        sample.discovered.as_mut().unwrap().unit = "invocation rows".to_string();
        sample.processed.as_mut().unwrap().value = Some(101);
        refresh_subset_relation(&mut sample);
        assert_eq!(sample.subset_relation, "incompatible");
        observation.finish_with_status("success");
    }

    #[test]
    fn scoped_committed_work_is_limited_to_its_managed_task() {
        let _guard = test_lock();
        clear_task_runtime_observation_for_tests();
        let observation = TaskExecutionObservation::begin(
            "raw_compression",
            "原始载荷压缩",
            "manual",
            Some("p2_derived"),
            "processing",
        );
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build observation test runtime")
            .block_on(with_managed_task_observation(observation.clone(), async {
                record_managed_task_processed_work(&["raw_compression"], 2);
                record_managed_task_processed_work(&["archive_upstream_activity_manifest"], 7);
            }));

        let sample = super::workload_sample("raw_compression").expect("root sample exists");
        assert_eq!(
            sample.processed.as_ref().and_then(|metric| metric.value),
            Some(2)
        );
        observation.finish_with_status("failed");
    }
}
