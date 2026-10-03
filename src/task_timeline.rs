use crate::{
    Utc, format_utc_iso_millis,
    maintenance_store::{MaintenanceStore, TimelineCoverage, TimelineSegment},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{broadcast, mpsc},
    task::JoinHandle,
    time::MissedTickBehavior,
};
use tokio_util::sync::CancellationToken;

const EVENT_CAPACITY: usize = 4096;
const BATCH_SIZE: usize = 256;
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const SHUTDOWN_FLUSH_ATTEMPTS: usize = 3;
const SHUTDOWN_FLUSH_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskObservationChange {
    Runtime,
    Timeline,
    Workload(String),
}

fn change_sender() -> &'static broadcast::Sender<TaskObservationChange> {
    static SENDER: OnceLock<broadcast::Sender<TaskObservationChange>> = OnceLock::new();
    SENDER.get_or_init(|| broadcast::channel(EVENT_CAPACITY).0)
}

pub(crate) fn subscribe_changes() -> broadcast::Receiver<TaskObservationChange> {
    change_sender().subscribe()
}

pub(crate) fn notify_runtime_changed() {
    let _ = change_sender().send(TaskObservationChange::Runtime);
}

fn notify_timeline_changed() {
    let _ = change_sender().send(TaskObservationChange::Timeline);
}

#[derive(Debug, Clone)]
pub(crate) enum TimelineEvent {
    ExecutionStarted {
        id: String,
        task_key: String,
        title: String,
        trigger_kind: String,
        execution_class: Option<String>,
        started_at: String,
        managed_run_id: Option<i64>,
    },
    ExecutionFinished {
        id: String,
        finished_at: String,
        duration_ms: u64,
        status: String,
    },
    ExecutionUnknown {
        id: String,
        last_observed_at: String,
    },
    ExecutionChildChanged {
        id: String,
        task_key: Option<String>,
        title: Option<String>,
    },
    CoverageGap {
        id: String,
        started_at: String,
        finished_at: String,
        reason: String,
    },
    DeferralStarted {
        id: String,
        task_key: String,
        reason: String,
        retry_at: Option<String>,
        started_at: String,
    },
    DeferralFinished {
        id: String,
        finished_at: String,
    },
    WorkloadSampleChanged {
        sample: Box<crate::maintenance_store::TaskWorkloadSample>,
    },
}

#[derive(Clone)]
struct EventSender {
    session_id: String,
    tx: mpsc::Sender<TimelineEvent>,
    dropped: Arc<AtomicU64>,
    dropped_interval: Arc<Mutex<Option<DroppedInterval>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DroppedInterval {
    id: String,
    started_at: String,
    finished_at: String,
}

fn record_dropped_event(dropped: &AtomicU64, interval: &Mutex<Option<DroppedInterval>>) {
    dropped.fetch_add(1, Ordering::Relaxed);
    let now = format_utc_iso_millis(Utc::now());
    let Ok(mut interval) = interval.lock() else {
        return;
    };
    match interval.as_mut() {
        Some(interval) => interval.finished_at = now,
        None => {
            *interval = Some(DroppedInterval {
                id: nanoid::nanoid!(),
                started_at: now.clone(),
                finished_at: now,
            });
        }
    }
}

fn clear_persisted_dropped_interval(
    interval: &Mutex<Option<DroppedInterval>>,
    persisted: &DroppedInterval,
) {
    if let Ok(mut interval) = interval.lock()
        && interval.as_ref() == Some(persisted)
    {
        *interval = None;
    }
}

fn event_sender() -> &'static Mutex<Option<EventSender>> {
    static SENDER: OnceLock<Mutex<Option<EventSender>>> = OnceLock::new();
    SENDER.get_or_init(|| Mutex::new(None))
}

fn active_deferrals() -> &'static Mutex<HashMap<String, (String, String)>> {
    static DEFERRALS: OnceLock<Mutex<HashMap<String, (String, String)>>> = OnceLock::new();
    DEFERRALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn recorder_handle() -> &'static Mutex<Option<JoinHandle<()>>> {
    static HANDLE: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
    HANDLE.get_or_init(|| Mutex::new(None))
}

fn recorder_cancel() -> &'static Mutex<Option<CancellationToken>> {
    static CANCEL: OnceLock<Mutex<Option<CancellationToken>>> = OnceLock::new();
    CANCEL.get_or_init(|| Mutex::new(None))
}

fn send_event(event: TimelineEvent) {
    let sender = event_sender().lock().ok().and_then(|sender| sender.clone());
    let Some(sender) = sender else { return };
    enqueue_event(&sender, event);
}

fn enqueue_event(sender: &EventSender, event: TimelineEvent) {
    if sender.tx.try_send(event).is_err() {
        record_dropped_event(&sender.dropped, &sender.dropped_interval);
    }
}

pub(crate) fn execution_started(
    id: String,
    task_key: String,
    title: String,
    trigger_kind: String,
    execution_class: Option<String>,
    started_at: String,
    managed_run_id: Option<i64>,
) {
    send_event(TimelineEvent::ExecutionStarted {
        id,
        task_key,
        title,
        trigger_kind,
        execution_class,
        started_at,
        managed_run_id,
    });
    notify_runtime_changed();
}

pub(crate) fn execution_finished(id: String, finished_at: String, duration_ms: u64, status: &str) {
    send_event(TimelineEvent::ExecutionFinished {
        id,
        finished_at,
        duration_ms,
        status: status.to_string(),
    });
    notify_runtime_changed();
}

pub(crate) fn execution_child_changed(id: String, task_key: Option<String>, title: Option<String>) {
    send_event(TimelineEvent::ExecutionChildChanged {
        id,
        task_key,
        title,
    });
    notify_runtime_changed();
}

pub(crate) fn execution_unknown(id: String, last_observed_at: String) {
    send_event(TimelineEvent::ExecutionUnknown {
        id,
        last_observed_at,
    });
    notify_runtime_changed();
}

pub(crate) fn workload_sample_changed(sample: crate::maintenance_store::TaskWorkloadSample) {
    let task_key = sample.task_key.clone();
    send_event(TimelineEvent::WorkloadSampleChanged {
        sample: Box::new(sample),
    });
    notify_runtime_changed();
    let _ = change_sender().send(TaskObservationChange::Workload(task_key));
}

fn notify_persisted_workload_changes(events: &[TimelineEvent]) {
    for event in events {
        if let TimelineEvent::WorkloadSampleChanged { sample } = event {
            let _ = change_sender().send(TaskObservationChange::Workload(sample.task_key.clone()));
        }
    }
}

pub(crate) fn note_background_denial(
    source: &'static str,
    reason: &str,
    retry_after_ms: Option<u64>,
) {
    let Some(task_key) = task_key_for_pressure_source(source) else {
        return;
    };
    let now = Utc::now();
    let started_at = format_utc_iso_millis(now);
    let retry_at = retry_after_ms.map(|delay| {
        format_utc_iso_millis(
            now + chrono::Duration::milliseconds(delay.min(i64::MAX as u64) as i64),
        )
    });
    let Ok(mut active) = active_deferrals().lock() else {
        return;
    };
    if let Some((id, existing_reason)) = active.get(source) {
        if existing_reason == reason {
            return;
        }
        send_event(TimelineEvent::DeferralFinished {
            id: id.clone(),
            finished_at: started_at.clone(),
        });
    }
    let id = nanoid::nanoid!();
    active.insert(source.to_string(), (id.clone(), reason.to_string()));
    send_event(TimelineEvent::DeferralStarted {
        id,
        task_key: task_key.to_string(),
        reason: reason.to_string(),
        retry_at,
        started_at,
    });
}

pub(crate) fn clear_background_denial(source: &'static str) {
    let id = active_deferrals()
        .lock()
        .ok()
        .and_then(|mut active| active.remove(source).map(|(id, _)| id));
    if let Some(id) = id {
        send_event(TimelineEvent::DeferralFinished {
            id,
            finished_at: format_utc_iso_millis(Utc::now()),
        });
    }
}

fn task_key_for_pressure_source(source: &str) -> Option<&'static str> {
    Some(match source {
        "dashboard_runtime_projection_reconcile" => "dashboard_runtime_projection_reconcile",
        "startup_hourly_rollup_bootstrap" | "startup_hourly_rollup_bootstrap_task_history" => {
            "startup_hourly_rollup_bootstrap"
        }
        "timeseries_minute_projection_flush" => "timeseries_minute_projection",
        "prompt_cache_topic_reconcile" | "dashboard_working_conversation_key_hydrate" => {
            "prompt_cache_materialization"
        }
        "system_raw_metrics_inventory" => "raw_payload_metrics_inventory",
        "invocation_timeline_cleanup" => "invocation_timeline_snapshot",
        "long_term_projection" | "long_term_projection_materialize" => "long_term_projection",
        "long_term_projection_write" => "long_term_projection",
        "summary_snapshot" => "summary_snapshot",
        "summary_coverage_recovery" => "summary_coverage_recovery",
        "summary_historical_coverage_recovery" | "summary_live_tail_reconciliation" => {
            "summary_coverage_recovery"
        }
        "retention_archive" | "retention" => "retention_archive",
        "data_retention_maintenance" | "system_task_run_retention" => "retention_archive",
        "raw_orphan_sweep" | "raw_sweep_directory_probe" => "retention_archive",
        "hourly_rollup_refresh" => "startup_hourly_rollup_bootstrap",
        "upstream_account_maintenance" => "upstream_account_maintenance",
        "account_activity_v2_priority_repair" => "startup_backfill",
        source if source.starts_with("startup_backfill.") => "startup_backfill",
        _ => return None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineCursor {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) watermark: i64,
    pub(crate) offset: u64,
    pub(crate) after_revision: Option<i64>,
    pub(crate) expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskTimelinePage {
    pub(crate) observed_at: String,
    pub(crate) window_start: String,
    pub(crate) window_end: String,
    pub(crate) watermark: i64,
    pub(crate) segments: Vec<TimelineSegment>,
    pub(crate) coverage: Vec<TimelineCoverage>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) reset_required: bool,
}

pub(crate) async fn start_recorder(store: Arc<MaintenanceStore>) {
    let session_id = nanoid::nanoid!();
    let (tx, rx) = mpsc::channel(EVENT_CAPACITY);
    let dropped = Arc::new(AtomicU64::new(0));
    let dropped_interval = Arc::new(Mutex::new(None));
    if let Ok(mut sender) = event_sender().lock() {
        *sender = Some(EventSender {
            session_id: session_id.clone(),
            tx,
            dropped: dropped.clone(),
            dropped_interval: dropped_interval.clone(),
        });
    }
    let shutdown = CancellationToken::new();
    if let Ok(mut cancel) = recorder_cancel().lock() {
        *cancel = Some(shutdown.clone());
    }
    let handle = tokio::spawn(run_recorder(
        store,
        session_id,
        rx,
        dropped,
        dropped_interval,
        shutdown,
    ));
    if let Ok(mut slot) = recorder_handle().lock() {
        *slot = Some(handle);
    }
}

pub(crate) async fn drain_after_shutdown() {
    if let Ok(mut cancel) = recorder_cancel().lock()
        && let Some(cancel) = cancel.take()
    {
        cancel.cancel();
    }
    let handle = recorder_handle()
        .lock()
        .ok()
        .and_then(|mut handle| handle.take());
    if let Some(handle) = handle
        && let Err(error) = handle.await
    {
        tracing::warn!(%error, "task timeline recorder did not shut down cleanly");
    }
}

async fn run_recorder(
    store: Arc<MaintenanceStore>,
    session_id: String,
    mut rx: mpsc::Receiver<TimelineEvent>,
    dropped: Arc<AtomicU64>,
    dropped_interval: Arc<Mutex<Option<DroppedInterval>>>,
    shutdown: CancellationToken,
) {
    let started_at = loop {
        let started_at = format_utc_iso_millis(Utc::now());
        match store.start_timeline_session(&session_id, &started_at).await {
            Ok(()) => break started_at,
            Err(error) => {
                tracing::warn!(%error, "task timeline recorder could not start");
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        if let Ok(mut sender) = event_sender().lock()
                            && sender.as_ref().is_some_and(|sender| sender.session_id == session_id)
                        {
                            *sender = None;
                        }
                        return;
                    }
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            }
        }
    };
    notify_timeline_changed();
    let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_heartbeat = tokio::time::Instant::now();
    let mut pending = Vec::with_capacity(BATCH_SIZE);
    let mut last_dropped = 0_u64;
    let mut last_success_at = started_at.clone();
    let mut coverage_gap: Option<(String, String)> = None;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            event = rx.recv() => match event {
                Some(event) if pending.len() < EVENT_CAPACITY => pending.push(event),
                Some(_) => record_dropped_event(&dropped, &dropped_interval),
                None => break,
            },
            _ = ticker.tick() => {
                while pending.len() < BATCH_SIZE {
                    match rx.try_recv() {
                        Ok(event) => pending.push(event),
                        Err(_) => break,
                    }
                }
                let dropped_total = dropped.load(Ordering::Relaxed);
                let heartbeat = last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL;
                if !pending.is_empty() || heartbeat || dropped_total != last_dropped {
                    let now = format_utc_iso_millis(Utc::now());
                    let mut events = pending.clone();
                    let dropped_gap = dropped_interval.lock().ok().and_then(|gap| gap.clone());
                    if let Some(gap) = &dropped_gap {
                        events.push(TimelineEvent::CoverageGap {
                            id: gap.id.clone(),
                            started_at: gap.started_at.clone(),
                            finished_at: gap.finished_at.clone(),
                            reason: "event_channel_overflow".to_string(),
                        });
                    }
                    if let Some((gap_id, gap_started_at)) = &coverage_gap {
                        events.push(TimelineEvent::CoverageGap {
                            id: gap_id.clone(),
                            started_at: gap_started_at.clone(),
                            finished_at: now.clone(),
                            reason: "maintenance_store_write_unavailable".to_string(),
                        });
                    }
                    match store.write_timeline_batch(
                        &session_id,
                        &events,
                        dropped_total,
                        &now,
                        heartbeat,
                    ).await {
                        Ok(()) => {
                            notify_timeline_changed();
                            notify_persisted_workload_changes(&events);
                            if events.iter().any(|event| matches!(
                                event,
                                TimelineEvent::DeferralStarted { .. }
                                    | TimelineEvent::DeferralFinished { .. }
                            )) {
                                notify_runtime_changed();
                            }
                            pending.clear();
                            if let Some(gap) = dropped_gap.as_ref() {
                                clear_persisted_dropped_interval(&dropped_interval, gap);
                            }
                            last_success_at = now;
                            coverage_gap = None;
                            last_dropped = dropped_total;
                            if heartbeat { last_heartbeat = tokio::time::Instant::now(); }
                        }
                        Err(error) => {
                            coverage_gap.get_or_insert_with(|| {
                                (nanoid::nanoid!(), last_success_at.clone())
                            });
                            tracing::warn!(%error, "task timeline batch could not be persisted");
                        }
                    }
                }
            }
        }
    }
    while pending.len() < EVENT_CAPACITY {
        match rx.try_recv() {
            Ok(event) => pending.push(event),
            Err(_) => break,
        }
    }
    while rx.try_recv().is_ok() {
        record_dropped_event(&dropped, &dropped_interval);
    }
    let now = format_utc_iso_millis(Utc::now());
    let dropped_total = dropped.load(Ordering::Relaxed);
    let mut events = pending.clone();
    let dropped_gap = dropped_interval.lock().ok().and_then(|gap| gap.clone());
    if let Some(gap) = &dropped_gap {
        events.push(TimelineEvent::CoverageGap {
            id: gap.id.clone(),
            started_at: gap.started_at.clone(),
            finished_at: gap.finished_at.clone(),
            reason: "event_channel_overflow".to_string(),
        });
    }
    if let Some((gap_id, gap_started_at)) = &coverage_gap {
        events.push(TimelineEvent::CoverageGap {
            id: gap_id.clone(),
            started_at: gap_started_at.clone(),
            finished_at: now.clone(),
            reason: "maintenance_store_write_unavailable".to_string(),
        });
    }
    let mut persisted = false;
    for attempt in 0..SHUTDOWN_FLUSH_ATTEMPTS {
        match store
            .write_timeline_batch(&session_id, &events, dropped_total, &now, true)
            .await
        {
            Ok(()) => {
                persisted = true;
                notify_persisted_workload_changes(&events);
                break;
            }
            Err(error) if attempt + 1 < SHUTDOWN_FLUSH_ATTEMPTS => {
                tracing::warn!(%error, attempt = attempt + 1, "task timeline shutdown flush failed; retrying");
                tokio::time::sleep(SHUTDOWN_FLUSH_RETRY_DELAY).await;
            }
            Err(error) => {
                tracing::error!(%error, "task timeline shutdown flush failed; leaving coverage open for restart recovery");
            }
        }
    }
    if persisted {
        notify_timeline_changed();
        if events.iter().any(|event| {
            matches!(
                event,
                TimelineEvent::DeferralStarted { .. } | TimelineEvent::DeferralFinished { .. }
            )
        }) {
            notify_runtime_changed();
        }
        if let Some(gap) = dropped_gap.as_ref() {
            clear_persisted_dropped_interval(&dropped_interval, gap);
        }
    } else {
        tracing::warn!(
            session_id,
            "task timeline coverage remains open so the next recorder can expose the unobserved interval"
        );
    }
    if persisted && let Err(error) = store.finish_timeline_session(&session_id, &now).await {
        tracing::warn!(%error, "task timeline session close could not be persisted");
    }
    if let Ok(mut sender) = event_sender().lock()
        && sender
            .as_ref()
            .is_some_and(|sender| sender.session_id == session_id)
    {
        *sender = None;
    }
}

pub(crate) async fn timeline_page(
    store: &MaintenanceStore,
    cursor: Option<&str>,
    after_revision: Option<i64>,
    from: Option<&str>,
    to: Option<&str>,
    limit: usize,
) -> anyhow::Result<TaskTimelinePage> {
    let now = Utc::now();
    let decoded_cursor = cursor.map(decode_cursor).transpose()?;
    let mut reset_required = false;
    let cursor = match decoded_cursor {
        Some(cursor) if DateTimeString::is_expired(&cursor.expires_at, now) => {
            reset_required = true;
            None
        }
        cursor => cursor,
    };
    let (window_start, window_end, watermark, offset, delta_from) = if let Some(cursor) = cursor {
        validate_decoded_cursor(&cursor)?;
        (
            cursor.from,
            cursor.to,
            cursor.watermark,
            cursor.offset,
            cursor.after_revision,
        )
    } else {
        let end = to
            .map(str::to_string)
            .unwrap_or_else(|| format_utc_iso_millis(now));
        let end_at = chrono::DateTime::parse_from_rfc3339(&end)?.with_timezone(&Utc);
        let start = from
            .map(str::to_string)
            .unwrap_or_else(|| format_utc_iso_millis(end_at - chrono::Duration::hours(24)));
        let watermark = store.timeline_revision().await?;
        (
            start,
            end,
            watermark,
            0,
            after_revision.filter(|revision| *revision >= 0),
        )
    };
    if reset_required {
        let watermark = store.timeline_revision().await?;
        return Ok(TaskTimelinePage {
            observed_at: format_utc_iso_millis(now),
            window_start: format_utc_iso_millis(now - chrono::Duration::hours(24)),
            window_end: format_utc_iso_millis(now),
            watermark,
            segments: Vec::new(),
            coverage: store.list_timeline_coverage().await?,
            next_cursor: None,
            reset_required: true,
        });
    }
    let segments = store
        .list_timeline_segments(
            &window_start,
            &window_end,
            watermark,
            delta_from,
            offset,
            limit,
        )
        .await?;
    let next_cursor = if segments.len() == limit {
        Some(encode_cursor(&TimelineCursor {
            from: window_start.clone(),
            to: window_end.clone(),
            watermark,
            offset: offset.saturating_add(limit as u64),
            after_revision: delta_from,
            expires_at: format_utc_iso_millis(now + chrono::Duration::minutes(10)),
        })?)
    } else {
        None
    };
    Ok(TaskTimelinePage {
        observed_at: format_utc_iso_millis(now),
        window_start,
        window_end,
        watermark,
        segments,
        coverage: store.list_timeline_coverage().await?,
        next_cursor,
        reset_required: false,
    })
}

struct DateTimeString;

impl DateTimeString {
    fn is_expired(value: &str, now: chrono::DateTime<Utc>) -> bool {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|expires| expires.with_timezone(&Utc) < now)
            .unwrap_or(true)
    }
}

fn encode_cursor(cursor: &TimelineCursor) -> anyhow::Result<String> {
    let payload = serde_json::to_vec(cursor)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload))
}

pub(crate) fn validate_cursor(raw: &str) -> anyhow::Result<()> {
    let cursor = decode_cursor(raw)?;
    validate_decoded_cursor(&cursor)
}

fn validate_decoded_cursor(cursor: &TimelineCursor) -> anyhow::Result<()> {
    use anyhow::ensure;
    let from = chrono::DateTime::parse_from_rfc3339(&cursor.from)?;
    let to = chrono::DateTime::parse_from_rfc3339(&cursor.to)?;
    let duration = to.signed_duration_since(from);
    ensure!(
        duration >= chrono::Duration::zero() && duration <= chrono::Duration::hours(24),
        "timeline cursor window must be between zero and 24 hours"
    );
    ensure!(
        cursor.watermark >= 0,
        "timeline cursor watermark is invalid"
    );
    ensure!(
        cursor.offset <= 1_000_000,
        "timeline cursor offset is invalid"
    );
    if let Some(revision) = cursor.after_revision {
        ensure!(revision >= 0, "timeline cursor revision is invalid");
    }
    Ok(())
}

fn decode_cursor(raw: &str) -> anyhow::Result<TimelineCursor> {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw)?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_retention_sources_resolve_to_the_managed_task() {
        assert_eq!(
            task_key_for_pressure_source("data_retention_maintenance"),
            Some("retention_archive")
        );
        assert_eq!(
            task_key_for_pressure_source("system_task_run_retention"),
            Some("retention_archive")
        );
    }

    #[test]
    fn dropped_events_accumulate_only_while_the_gap_is_open() {
        let dropped = AtomicU64::new(0);
        let interval = Mutex::new(None);
        record_dropped_event(&dropped, &interval);
        let first = interval
            .lock()
            .expect("lock drop interval")
            .clone()
            .unwrap();
        record_dropped_event(&dropped, &interval);
        let extended = interval
            .lock()
            .expect("lock drop interval")
            .clone()
            .unwrap();

        assert_eq!(dropped.load(Ordering::Relaxed), 2);
        assert_eq!(first.id, extended.id);
        assert_eq!(first.started_at, extended.started_at);
        assert!(extended.finished_at >= first.finished_at);

        clear_persisted_dropped_interval(&interval, &extended);
        assert!(interval.lock().expect("lock drop interval").is_none());
        record_dropped_event(&dropped, &interval);
        let next = interval
            .lock()
            .expect("lock drop interval")
            .clone()
            .unwrap();
        assert_ne!(next.id, extended.id);
    }

    #[test]
    fn full_recorder_queue_drops_workload_events_and_marks_a_gap() {
        let (tx, _rx) = mpsc::channel(1);
        tx.try_send(TimelineEvent::ExecutionUnknown {
            id: "queued-event".to_string(),
            last_observed_at: "2026-10-03T00:00:00.000Z".to_string(),
        })
        .expect("fill recorder queue");
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_interval = Arc::new(Mutex::new(None));
        let sender = EventSender {
            session_id: "saturated-recorder".to_string(),
            tx,
            dropped: dropped.clone(),
            dropped_interval: dropped_interval.clone(),
        };
        let sample = crate::maintenance_store::TaskWorkloadSample {
            sample_id: "workload-sample".to_string(),
            execution_uid: "execution".to_string(),
            managed_run_id: None,
            task_key: "retention_archive".to_string(),
            trigger_kind: "manual".to_string(),
            attempted_at: "2026-10-03T00:00:00.000Z".to_string(),
            actual_started_at: Some("2026-10-03T00:00:00.001Z".to_string()),
            finished_at: None,
            status: "running".to_string(),
            reason: None,
            sequence: 1,
            pending: None,
            discovered: None,
            processed: None,
            subset_relation: "unknown".to_string(),
        };

        enqueue_event(
            &sender,
            TimelineEvent::WorkloadSampleChanged {
                sample: Box::new(sample),
            },
        );

        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        assert!(
            dropped_interval
                .lock()
                .expect("read recorder coverage gap")
                .is_some()
        );
    }
}
