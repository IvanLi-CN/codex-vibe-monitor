use super::*;
use crate::db_pressure::{DbPressureDenyReason, DbPressureGate};
use axum::http::header;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde::ser::{SerializeSeq, SerializeStruct};
use serde_json::json;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::Notify;

const SUBSCRIPTION_REPLAY_WINDOW_SECS: i64 = 60;
const SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC: usize = 512;
const SUBSCRIPTION_REPLAY_MAX_BYTES_PER_TOPIC: usize = 1024 * 1024;
const SUBSCRIPTION_REPLAY_MAX_GAP_EVENTS: usize = 128;
const SUBSCRIPTION_REPLAY_MAX_GAP_BYTES: usize = 256 * 1024;
const SUBSCRIPTION_DEFAULT_TIME_ZONE: &str = "Asia/Shanghai";
const SUBSCRIPTION_DEFAULT_DASHBOARD_RECENT_LIMIT: i64 = 16;
const SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT: i64 = 16;
const SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES: i64 = 5;
const SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE: i64 = 20;
const SUBSCRIPTION_DEFAULT_INVOCATION_LIMIT: i64 = 20;
const SUBSCRIPTION_CONVERSATION_HISTORY_LIMIT: i64 = 50;
const SUBSCRIPTION_CONVERSATION_OPERATION_LIMIT: usize = 20;
const SUBSCRIPTION_CONVERSATION_OVERVIEW_MAX_RECORDS: usize = 1_000;
#[cfg(not(test))]
const DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
const DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(not(test))]
const DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL: Duration = Duration::from_secs(5);
#[cfg(test)]
const DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL: Duration = Duration::from_millis(500);
const SUMMARY_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);
const PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE: Duration = Duration::from_secs(1);
#[cfg(test)]
const PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE: Duration = Duration::from_millis(50);
const PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
const RUNTIME_TOPIC_RECOVERY_QUEUE_CAPACITY: usize = 64;
const RUNTIME_TOPIC_RECOVERY_BATCH_SIZE: usize = 8;
const RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF: Duration = Duration::from_secs(1);
const SUBSCRIPTION_INITIAL_TOPIC_BUILD_ATTEMPTS: usize = 3;
#[cfg(test)]
const DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON: &str = "dashboard-runtime-topology-contract";
#[cfg(not(test))]
const CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_secs(2);
#[cfg(test)]
const CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(50);

fn subscription_calendar_anchor(topic: &SubscriptionTopic) -> Option<String> {
    let SubscriptionTopic::SummaryCurrent {
        window, time_zone, ..
    } = topic
    else {
        return None;
    };
    if !matches!(window.as_str(), "yesterday" | "previous7d") {
        return None;
    }
    let reporting_tz = parse_reporting_tz(Some(time_zone.as_str())).ok()?;
    Some(
        Utc::now()
            .with_timezone(&reporting_tz)
            .date_naive()
            .to_string(),
    )
}

fn subscription_calendar_rollover_delay(topic: &SubscriptionTopic) -> Duration {
    let SubscriptionTopic::SummaryCurrent { time_zone, .. } = topic else {
        return Duration::from_secs(1);
    };
    let reporting_tz = parse_reporting_tz(Some(time_zone.as_str()))
        .unwrap_or_else(|_| "Asia/Shanghai".parse().expect("default timezone is valid"));
    subscription_calendar_rollover_delay_at(topic, Utc::now(), reporting_tz)
}

fn subscription_calendar_rollover_delay_at(
    topic: &SubscriptionTopic,
    now: DateTime<Utc>,
    reporting_tz: Tz,
) -> Duration {
    let SubscriptionTopic::SummaryCurrent { .. } = topic else {
        return Duration::from_secs(1);
    };
    let tomorrow = now.with_timezone(&reporting_tz).date_naive() + ChronoDuration::days(1);
    let next_midnight = local_midnight_utc(tomorrow, reporting_tz);
    (next_midnight - now)
        .to_std()
        .unwrap_or_else(|_| Duration::from_millis(1))
        .max(Duration::from_millis(1))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionTopicDescriptor {
    pub(crate) topic: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionResumeCursor {
    pub(crate) topic_key: String,
    pub(crate) cursor: u64,
    pub(crate) schema_epoch: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SubscriptionCompactResumeCursor {
    topic_index: usize,
    cursor: u64,
    schema_epoch: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum SubscriptionResumeCursorQuery {
    Legacy(SubscriptionResumeCursor),
    Compact(SubscriptionCompactResumeCursor),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum SubscriptionEventEnvelope {
    Snapshot {
        topic: SubscriptionTopicDescriptor,
        #[serde(rename = "topicKey")]
        topic_key: String,
        #[serde(rename = "schemaEpoch")]
        schema_epoch: String,
        cursor: u64,
        payload: Value,
    },
    Replay {
        topic: SubscriptionTopicDescriptor,
        #[serde(rename = "topicKey")]
        topic_key: String,
        #[serde(rename = "schemaEpoch")]
        schema_epoch: String,
        cursor: u64,
        payload: Value,
    },
    Live {
        topic: SubscriptionTopicDescriptor,
        #[serde(rename = "topicKey")]
        topic_key: String,
        #[serde(rename = "schemaEpoch")]
        schema_epoch: String,
        cursor: u64,
        payload: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubscriptionStreamQuery {
    pub(crate) topics: Option<String>,
    pub(crate) resume: Option<String>,
    pub(crate) attempt: Option<u64>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum TopicFrameKind {
    Snapshot,
    Replay,
    Live,
}

#[derive(Debug)]
pub(crate) struct SerializedTopicFrame {
    pub(crate) topic_key: String,
    pub(crate) schema_epoch: String,
    pub(crate) cursor: u64,
    descriptor: SubscriptionTopicDescriptor,
    fingerprint: u64,
    payload_bytes: Bytes,
    envelope_metadata_bytes: Bytes,
}

impl SerializedTopicFrame {
    fn event_chunks(&self, kind: TopicFrameKind) -> [Bytes; 4] {
        let prefix = match kind {
            TopicFrameKind::Snapshot => Bytes::from_static(b"data: {\"type\":\"snapshot"),
            TopicFrameKind::Replay => Bytes::from_static(b"data: {\"type\":\"replay"),
            TopicFrameKind::Live => Bytes::from_static(b"data: {\"type\":\"live"),
        };
        [
            prefix,
            self.envelope_metadata_bytes.clone(),
            self.payload_bytes.clone(),
            Bytes::from_static(b"}\n\n"),
        ]
    }

    fn retained_bytes(&self) -> usize {
        self.envelope_metadata_bytes.len() + self.payload_bytes.len()
    }

    #[cfg(test)]
    pub(crate) fn payload_value(&self) -> Value {
        serde_json::from_slice(&self.payload_bytes).expect("serialized topic payload")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardTopicTopologyCounterSnapshot {
    pub(crate) active_subscriber_count: u64,
    pub(crate) builder_count: u64,
    #[serde(skip)]
    pub(crate) generic_fallback_build_count: u64,
    #[serde(skip)]
    pub(crate) live_path_db_read_count: u64,
    pub(crate) materialization_count: u64,
    pub(crate) serialization_count: u64,
    pub(crate) payload_clone_count: u64,
    pub(crate) frame_bytes_count: u64,
    pub(crate) frame_reused: u64,
    pub(crate) cursor_advanced: u64,
    pub(crate) lagged_count: u64,
    pub(crate) skipped_count: u64,
    #[serde(skip)]
    pub(crate) reconnect_churn_count: u64,
    pub(crate) business_payload_count: u64,
    pub(crate) json_overlay_count: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardDeliveryTopologyCounterSnapshot {
    pub(crate) activity: DashboardTopicTopologyCounterSnapshot,
    pub(crate) summary: DashboardTopicTopologyCounterSnapshot,
    pub(crate) network_timeseries: DashboardTopicTopologyCounterSnapshot,
    pub(crate) network_recent: DashboardTopicTopologyCounterSnapshot,
    #[serde(skip)]
    pub(crate) working_conversations: DashboardTopicTopologyCounterSnapshot,
    #[serde(skip)]
    pub(crate) parallel_work: DashboardTopicTopologyCounterSnapshot,
    #[serde(skip)]
    pub(crate) timeseries: DashboardTopicTopologyCounterSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubscriptionTopicClass {
    HotProjection,
    ClosedSnapshot,
    BoundedColdHydrate,
}

impl SubscriptionTopicClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::HotProjection => "hot_projection",
            Self::ClosedSnapshot => "closed_snapshot",
            Self::BoundedColdHydrate => "bounded_cold_hydrate",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardHotTopicHealthSnapshot {
    pub(crate) topic_class: String,
    pub(crate) state: String,
    pub(crate) active_subscriber_count: u64,
    pub(crate) builder_count: u64,
    pub(crate) generic_fallback_build_count: u64,
    pub(crate) live_path_db_read_count: u64,
    pub(crate) materialization_count: u64,
    pub(crate) serialization_count: u64,
    pub(crate) payload_clone_count: u64,
    pub(crate) frame_reused: u64,
    pub(crate) cadence_miss_count: u64,
    pub(crate) reconnect_churn_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardHotTopicsHealthSnapshot {
    pub(crate) state: String,
    pub(crate) activity: DashboardHotTopicHealthSnapshot,
    pub(crate) summary: DashboardHotTopicHealthSnapshot,
    pub(crate) network_timeseries: DashboardHotTopicHealthSnapshot,
    pub(crate) network_recent: DashboardHotTopicHealthSnapshot,
    pub(crate) working_conversations: DashboardHotTopicHealthSnapshot,
    pub(crate) parallel_work: DashboardHotTopicHealthSnapshot,
    pub(crate) timeseries: DashboardHotTopicHealthSnapshot,
}

#[derive(Debug, Clone, Copy, Default)]
struct DashboardHotTopicRecoveryState {
    degraded: bool,
    deferred: bool,
}

impl DashboardHotTopicRecoveryState {
    fn merge(&mut self, other: Self) {
        self.degraded |= other.degraded;
        self.deferred |= other.deferred;
    }
}

#[derive(Debug, Default)]
struct DashboardHotTopicRecoveryHealth {
    activity: DashboardHotTopicRecoveryState,
    summary: DashboardHotTopicRecoveryState,
    network_timeseries: DashboardHotTopicRecoveryState,
    network_recent: DashboardHotTopicRecoveryState,
    working_conversations: DashboardHotTopicRecoveryState,
    parallel_work: DashboardHotTopicRecoveryState,
    timeseries: DashboardHotTopicRecoveryState,
}

impl DashboardHotTopicRecoveryHealth {
    fn record(&mut self, topic: &SubscriptionTopic, state: DashboardHotTopicRecoveryState) {
        let target = match topic {
            SubscriptionTopic::DashboardActivityCurrent { .. } => &mut self.activity,
            SubscriptionTopic::SummaryCurrent { .. } => &mut self.summary,
            SubscriptionTopic::DashboardNetworkTimeseriesWindow { .. } => {
                &mut self.network_timeseries
            }
            SubscriptionTopic::DashboardNetworkRecentCurrent => &mut self.network_recent,
            SubscriptionTopic::DashboardWorkingConversationsCurrent { .. } => {
                &mut self.working_conversations
            }
            SubscriptionTopic::ParallelWorkCurrent { .. } => &mut self.parallel_work,
            SubscriptionTopic::TimeseriesOpenWindow { .. } => &mut self.timeseries,
            _ => return,
        };
        target.merge(state);
    }
}

#[derive(Debug, Default)]
struct DashboardTopicTopologyCounters {
    active_subscriber_count: AtomicU64,
    builder_count: AtomicU64,
    generic_fallback_build_count: AtomicU64,
    live_path_db_read_count: AtomicU64,
    materialization_count: AtomicU64,
    serialization_count: AtomicU64,
    payload_clone_count: AtomicU64,
    frame_bytes_count: AtomicU64,
    frame_reused: AtomicU64,
    cursor_advanced: AtomicU64,
    lagged_count: AtomicU64,
    skipped_count: AtomicU64,
    reconnect_churn_count: AtomicU64,
    business_payload_count: AtomicU64,
    json_overlay_count: AtomicU64,
}

impl DashboardTopicTopologyCounters {
    fn snapshot(&self) -> DashboardTopicTopologyCounterSnapshot {
        DashboardTopicTopologyCounterSnapshot {
            active_subscriber_count: self.active_subscriber_count.load(Ordering::Relaxed),
            builder_count: self.builder_count.load(Ordering::Relaxed),
            generic_fallback_build_count: self.generic_fallback_build_count.load(Ordering::Relaxed),
            live_path_db_read_count: self.live_path_db_read_count.load(Ordering::Relaxed),
            materialization_count: self.materialization_count.load(Ordering::Relaxed),
            serialization_count: self.serialization_count.load(Ordering::Relaxed),
            payload_clone_count: self.payload_clone_count.load(Ordering::Relaxed),
            frame_bytes_count: self.frame_bytes_count.load(Ordering::Relaxed),
            frame_reused: self.frame_reused.load(Ordering::Relaxed),
            cursor_advanced: self.cursor_advanced.load(Ordering::Relaxed),
            lagged_count: self.lagged_count.load(Ordering::Relaxed),
            skipped_count: self.skipped_count.load(Ordering::Relaxed),
            reconnect_churn_count: self.reconnect_churn_count.load(Ordering::Relaxed),
            business_payload_count: self.business_payload_count.load(Ordering::Relaxed),
            json_overlay_count: self.json_overlay_count.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        self.builder_count.store(0, Ordering::Relaxed);
        self.generic_fallback_build_count
            .store(0, Ordering::Relaxed);
        self.live_path_db_read_count.store(0, Ordering::Relaxed);
        self.materialization_count.store(0, Ordering::Relaxed);
        self.serialization_count.store(0, Ordering::Relaxed);
        self.payload_clone_count.store(0, Ordering::Relaxed);
        self.frame_bytes_count.store(0, Ordering::Relaxed);
        self.frame_reused.store(0, Ordering::Relaxed);
        self.cursor_advanced.store(0, Ordering::Relaxed);
        self.lagged_count.store(0, Ordering::Relaxed);
        self.skipped_count.store(0, Ordering::Relaxed);
        self.reconnect_churn_count.store(0, Ordering::Relaxed);
        self.business_payload_count.store(0, Ordering::Relaxed);
        self.json_overlay_count.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct DashboardDeliveryTopologyCounters {
    activity: DashboardTopicTopologyCounters,
    summary: DashboardTopicTopologyCounters,
    network_timeseries: DashboardTopicTopologyCounters,
    network_recent: DashboardTopicTopologyCounters,
    working_conversations: DashboardTopicTopologyCounters,
    parallel_work: DashboardTopicTopologyCounters,
    timeseries: DashboardTopicTopologyCounters,
    working_conversations_cadence_miss_count: AtomicU64,
    parallel_work_cadence_miss_count: AtomicU64,
}

impl DashboardDeliveryTopologyCounters {
    fn for_topic(&self, topic_name: &str) -> Option<&DashboardTopicTopologyCounters> {
        match topic_name {
            "dashboard.activity.current" => Some(&self.activity),
            "stats.summary.current" => Some(&self.summary),
            "dashboard.network-timeseries.window" => Some(&self.network_timeseries),
            "dashboard.network-recent.current" => Some(&self.network_recent),
            "dashboard.working-conversations.current" => Some(&self.working_conversations),
            "stats.parallel-work.current" => Some(&self.parallel_work),
            "stats.timeseries.open-window" => Some(&self.timeseries),
            _ => None,
        }
    }

    fn set_active_subscriber_count(&self, topic_name: &str, count: usize) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic
                .active_subscriber_count
                .store(count as u64, Ordering::Relaxed);
        }
    }

    fn record_materialization(&self, topic_name: &str, generic_fallback: bool) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.builder_count.fetch_add(1, Ordering::Relaxed);
            topic.materialization_count.fetch_add(1, Ordering::Relaxed);
            if generic_fallback {
                topic
                    .generic_fallback_build_count
                    .fetch_add(1, Ordering::Relaxed);
                topic
                    .live_path_db_read_count
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_serialization(&self, topic_name: &str, frame_bytes: usize) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.serialization_count.fetch_add(1, Ordering::Relaxed);
            topic
                .frame_bytes_count
                .fetch_add(frame_bytes as u64, Ordering::Relaxed);
        }
    }

    fn record_frame_reused(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.frame_reused.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_shared_frame_delivery(&self, topic_name: &str) {
        let Some(topic) = self.for_topic(topic_name) else {
            return;
        };
        if topic.active_subscriber_count.load(Ordering::Relaxed) > 1 {
            topic.frame_reused.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_cursor_advanced(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.cursor_advanced.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_lag(&self, topic_name: &str, skipped: u64) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.lagged_count.fetch_add(1, Ordering::Relaxed);
            topic.skipped_count.fetch_add(skipped, Ordering::Relaxed);
        }
    }

    fn record_reconnect_churn(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.reconnect_churn_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_business_payload(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.business_payload_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_json_overlay(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.json_overlay_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_cadence_miss(&self, topic_name: &str) {
        let counter = match topic_name {
            "dashboard.working-conversations.current" => {
                &self.working_conversations_cadence_miss_count
            }
            "stats.parallel-work.current" => &self.parallel_work_cadence_miss_count,
            _ => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> DashboardDeliveryTopologyCounterSnapshot {
        DashboardDeliveryTopologyCounterSnapshot {
            activity: self.activity.snapshot(),
            summary: self.summary.snapshot(),
            network_timeseries: self.network_timeseries.snapshot(),
            network_recent: self.network_recent.snapshot(),
            working_conversations: self.working_conversations.snapshot(),
            parallel_work: self.parallel_work.snapshot(),
            timeseries: self.timeseries.snapshot(),
        }
    }

    fn hot_topic_health(
        &self,
        projection: DashboardRuntimeTopologyCounterSnapshot,
        recovery: DashboardHotTopicRecoveryHealth,
    ) -> DashboardHotTopicsHealthSnapshot {
        let counters = self.snapshot();
        let topic = |counter: DashboardTopicTopologyCounterSnapshot,
                     cadence_miss_count: u64,
                     recovery: DashboardHotTopicRecoveryState| {
            let degraded = recovery.degraded
                || counter.generic_fallback_build_count > 0
                || counter.live_path_db_read_count > 0
                || counter.reconnect_churn_count > 0
                || counter.lagged_count > 0
                || counter.skipped_count > 0
                || counter.payload_clone_count > 0
                || counter.json_overlay_count > 0
                || cadence_miss_count > 0;
            DashboardHotTopicHealthSnapshot {
                topic_class: SubscriptionTopicClass::HotProjection.as_str().to_string(),
                state: if degraded {
                    "degraded"
                } else if recovery.deferred {
                    "deferred"
                } else {
                    "healthy"
                }
                .to_string(),
                active_subscriber_count: counter.active_subscriber_count,
                builder_count: counter.builder_count,
                generic_fallback_build_count: counter.generic_fallback_build_count,
                live_path_db_read_count: counter.live_path_db_read_count,
                materialization_count: counter.materialization_count,
                serialization_count: counter.serialization_count,
                payload_clone_count: counter.payload_clone_count,
                frame_reused: counter.frame_reused,
                cadence_miss_count,
                reconnect_churn_count: counter.reconnect_churn_count,
            }
        };
        let current_cadence = projection.current.cadence_miss_count;
        let network_cadence = projection.network.cadence_miss_count;
        let terminal_cadence = projection.terminal.cadence_miss_count;
        let working_conversations_cadence = self
            .working_conversations_cadence_miss_count
            .load(Ordering::Relaxed);
        let parallel_work_cadence = self
            .parallel_work_cadence_miss_count
            .load(Ordering::Relaxed);
        let activity_cadence = current_cadence
            .saturating_add(network_cadence)
            .saturating_add(terminal_cadence);
        let current_and_terminal_cadence = current_cadence.saturating_add(terminal_cadence);
        let mut health = DashboardHotTopicsHealthSnapshot {
            state: String::new(),
            activity: topic(counters.activity, activity_cadence, recovery.activity),
            summary: topic(
                counters.summary,
                current_and_terminal_cadence,
                recovery.summary,
            ),
            network_timeseries: topic(
                counters.network_timeseries,
                network_cadence,
                recovery.network_timeseries,
            ),
            network_recent: topic(
                counters.network_recent,
                network_cadence,
                recovery.network_recent,
            ),
            working_conversations: topic(
                counters.working_conversations,
                working_conversations_cadence,
                recovery.working_conversations,
            ),
            parallel_work: topic(
                counters.parallel_work,
                parallel_work_cadence,
                recovery.parallel_work,
            ),
            timeseries: topic(
                counters.timeseries,
                current_and_terminal_cadence,
                recovery.timeseries,
            ),
        };
        let states = [
            health.activity.state.as_str(),
            health.summary.state.as_str(),
            health.network_timeseries.state.as_str(),
            health.network_recent.state.as_str(),
            health.working_conversations.state.as_str(),
            health.parallel_work.state.as_str(),
            health.timeseries.state.as_str(),
        ];
        health.state = if states.contains(&"degraded") {
            "degraded"
        } else if states.contains(&"deferred") {
            "deferred"
        } else {
            "healthy"
        }
        .to_string();
        health
    }

    fn has_degraded_signal(&self) -> bool {
        let snapshot = self.snapshot();
        [
            snapshot.activity,
            snapshot.summary,
            snapshot.network_timeseries,
            snapshot.network_recent,
            snapshot.working_conversations,
            snapshot.parallel_work,
            snapshot.timeseries,
        ]
        .into_iter()
        .any(|topic| {
            topic.lagged_count > 0
                || topic.skipped_count > 0
                || topic.payload_clone_count > 0
                || topic.json_overlay_count > 0
                || topic.generic_fallback_build_count > 0
                || topic.live_path_db_read_count > 0
                || topic.reconnect_churn_count > 0
        })
    }

    #[cfg(test)]
    fn reset(&self) {
        self.activity.reset();
        self.summary.reset();
        self.network_timeseries.reset();
        self.network_recent.reset();
        self.working_conversations.reset();
        self.parallel_work.reset();
        self.timeseries.reset();
        self.working_conversations_cadence_miss_count
            .store(0, Ordering::Relaxed);
        self.parallel_work_cadence_miss_count
            .store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionDispatchEvent {
    pub(crate) frame: Arc<SerializedTopicFrame>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheTopicProjectionHealthSnapshot {
    pub(crate) mode: String,
    pub(crate) active_topic_count: u64,
    pub(crate) dirty_key_count: u64,
    pub(crate) dirty_last_good_topic_count: u64,
    pub(crate) pressure_deferred_topic_count: u64,
    pub(crate) failed_or_stale_topic_count: u64,
    pub(crate) recovery_state: String,
    pub(crate) coalesced_event_count: u64,
    pub(crate) full_hydration_count: u64,
    pub(crate) bounded_key_hydration_count: u64,
    pub(crate) bounded_cold_recovery_topic_count: u64,
    pub(crate) live_path_db_read_count: u64,
    pub(crate) baseline_age_ms: u64,
    pub(crate) response_source: String,
}

#[cfg(test)]
type DashboardTopologyFramesByTopic = BTreeMap<String, Vec<Arc<SerializedTopicFrame>>>;
#[cfg(test)]
type DashboardTopologySseFrameObservations = HashMap<u64, DashboardTopologyFramesByTopic>;

#[derive(Debug)]
pub(crate) struct SubscriptionHub {
    state: Mutex<SubscriptionHubState>,
    broadcaster: broadcast::Sender<SubscriptionDispatchEvent>,
    runtime_mutation_bus: Arc<RuntimeMutationBus>,
    runtime_topic_recovery_notify: Arc<Notify>,
    serialization_count: AtomicU64,
    dashboard_topology_counters: DashboardDeliveryTopologyCounters,
    #[cfg(test)]
    dashboard_topology_sse_frame_observations: Mutex<DashboardTopologySseFrameObservations>,
}

#[derive(Debug, Default)]
struct SubscriptionHubState {
    topics: HashMap<String, CachedSubscriptionTopic>,
    active_topics: HashMap<String, SubscriptionTopic>,
    active_topic_dependencies: HashMap<RuntimeTopicDependency, HashSet<String>>,
    active_subscribers: HashMap<String, usize>,
    active_topic_names: HashMap<String, usize>,
    dashboard_live_subscriber_count: usize,
    server_push_subscribers: HashMap<String, usize>,
    server_push_tasks: HashSet<String>,
    dashboard_current_slice: Option<Arc<DashboardCurrentProjectionSlice>>,
    dashboard_network_slice: Option<Arc<DashboardNetworkProjectionSlice>>,
    dashboard_terminal_slice: Option<Arc<DashboardTerminalProjectionSlice>>,
    prompt_cache_prebaseline_records: HashMap<String, BTreeMap<String, PromptCacheTopicDelta>>,
    prompt_cache_prebaseline_key_hydrations: HashMap<String, BTreeSet<String>>,
    parallel_work_prebaseline_mutations:
        HashMap<String, BTreeMap<String, RuntimeInvocationMutation>>,
    runtime_topic_recovery_generation: u64,
    runtime_topic_recovery_queue: VecDeque<(String, u64)>,
    runtime_topic_recovery_queued: HashSet<String>,
    runtime_topic_recovery_running: bool,
}

#[derive(Debug, Clone)]
struct CachedSubscriptionTopic {
    topic: SubscriptionTopic,
    descriptor: SubscriptionTopicDescriptor,
    schema_epoch: String,
    cursor: u64,
    snapshot_built_at: Instant,
    refresh_scheduled: bool,
    conversation_overview_refresh_scheduled: bool,
    conversation_overview_refresh_in_flight: bool,
    conversation_overview_refresh_pending: bool,
    dirty: bool,
    runtime_topic_recovery_generation: u64,
    runtime_topic_recovery_retry_at: Option<Instant>,
    summary_refresh_scheduled: bool,
    summary_refresh_in_flight: bool,
    summary_pending_event_count: u64,
    summary_retry_backoff_ms: u64,
    parallel_work_refresh_scheduled: bool,
    prompt_cache_refresh_scheduled: bool,
    prompt_cache_reconcile_scheduled: bool,
    prompt_cache_key_hydration_scheduled: bool,
    prompt_cache_pending_records: BTreeMap<String, PromptCacheTopicDelta>,
    prompt_cache_pending_key_hydrations: BTreeSet<String>,
    prompt_cache_candidate_refill_required: bool,
    prompt_cache_applied_terminal_ids: HashSet<String>,
    prompt_cache_coalesced_event_count: u64,
    prompt_cache_full_hydration_count: u64,
    prompt_cache_bounded_key_hydration_count: u64,
    prompt_cache_baseline_at: Option<Instant>,
    prompt_cache_baseline_row_id: i64,
    prompt_cache_response_source: &'static str,
    prompt_cache_reconcile_required: bool,
    prompt_cache_pressure_deferred: bool,
    latest_live_snapshot: Option<DashboardActivityLiveSnapshot>,
    calendar_anchor: Option<String>,
    continuity_reset_cursor: Option<u64>,
    dashboard_materializer: Option<DashboardTopicMaterializer>,
    dashboard_base_revision: u64,
    dashboard_materialized_revision: Option<DashboardTopicRevision>,
    snapshot_payload: Value,
    snapshot_frame: Arc<SerializedTopicFrame>,
    snapshot_bytes: usize,
    replay_events: VecDeque<ReplayableTopicEvent>,
    replay_bytes: usize,
}

#[derive(Debug, Clone)]
struct RuntimeTopicWork {
    topic: SubscriptionTopic,
    terminal_event_count: u64,
    includes_invocation_mutation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RuntimeTopicDependency {
    Invocation,
    ModelRouting,
    PromptCacheProjection,
    PromptCacheWindow,
    PromptCacheStickyWindow,
    DashboardWorkingConversationsProjection,
    Attempt(String),
    Binding(String),
    HistoryPromptCacheKey(String),
    HistoryStickyKey(String),
    StickyRoute(String),
}

#[derive(Debug, Clone, PartialEq)]
struct PromptCacheTopicDelta {
    row_id: i64,
    identity: String,
    invoke_id: String,
    prompt_cache_key: Option<String>,
    sticky_key: Option<String>,
    occurred_at: String,
    is_runtime_removed: bool,
    status: String,
    is_terminal: bool,
    is_success: bool,
    request_tokens: i64,
    cost: f64,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    preview: Option<PromptCacheConversationInvocationPreviewResponse>,
}

fn prompt_cache_hydration_changed_pending_keys(
    pending_at_hydration_start: &BTreeMap<String, PromptCacheTopicDelta>,
    pending_after_hydration: &BTreeMap<String, PromptCacheTopicDelta>,
    hydration_keys: &[String],
) -> BTreeSet<String> {
    pending_after_hydration
        .iter()
        .filter_map(|(identity, record)| {
            let key = record.prompt_cache_key.as_deref()?;
            (hydration_keys.iter().any(|candidate| candidate == key)
                && pending_at_hydration_start.get(identity) != Some(record))
            .then(|| key.to_string())
        })
        .collect()
}

struct PromptCacheBaselineBuild {
    baseline_row_id: i64,
    persisted_identities: HashSet<String>,
    runtime_overlay_terminal_identities: HashSet<String>,
}

struct ParallelWorkBaselineBuild {
    persisted_identities: HashSet<String>,
}

impl PromptCacheTopicDelta {
    #[cfg(test)]
    fn from_record(record: &ApiInvocation) -> Result<Option<Self>, ApiError> {
        Ok(PromptCacheRuntimeProjection::from_record(record)
            .as_ref()
            .and_then(Self::from_runtime_projection))
    }

    fn from_runtime_mutation(
        mutation: &RuntimeInvocationMutation,
        runtime_projection: Option<&PromptCacheRuntimeProjection>,
    ) -> Result<Option<Self>, ApiError> {
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            return Ok(Self::from_runtime_removal(mutation));
        }
        Ok(runtime_projection.and_then(Self::from_runtime_projection))
    }

    fn from_runtime_projection(projection: &PromptCacheRuntimeProjection) -> Option<Self> {
        let preview = projection.preview.clone();
        let status = preview.status.clone();
        Some(Self {
            row_id: projection.row_id,
            identity: format!("{}\0{}", preview.invoke_id, preview.occurred_at),
            invoke_id: preview.invoke_id.clone(),
            prompt_cache_key: projection.prompt_cache_key.clone(),
            sticky_key: projection.sticky_key.clone(),
            occurred_at: parse_to_utc_datetime(&preview.occurred_at)
                .map(format_utc_iso)
                .unwrap_or_else(|| preview.occurred_at.clone()),
            is_runtime_removed: false,
            is_terminal: prompt_invocation_status_counts_toward_terminal_totals(Some(&status)),
            is_success: prompt_invocation_status_is_success_like(
                Some(&status),
                preview.error_message.as_deref(),
            ),
            status,
            request_tokens: preview.total_tokens.max(0),
            cost: preview.cost.unwrap_or_default(),
            upstream_account_id: preview.upstream_account_id,
            upstream_account_name: preview.upstream_account_name.clone(),
            preview: Some(preview),
        })
    }

    fn from_runtime_removal(mutation: &RuntimeInvocationMutation) -> Option<Self> {
        if mutation.prompt_cache_key.is_none() && mutation.sticky_key.is_none() {
            return None;
        }
        let occurred_at = parse_to_utc_datetime(&mutation.identity.occurred_at)
            .map(format_utc_iso)
            .unwrap_or_else(|| mutation.identity.occurred_at.clone());
        Some(Self {
            row_id: mutation.row_id.unwrap_or_default(),
            identity: format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            ),
            invoke_id: mutation.identity.invoke_id.clone(),
            prompt_cache_key: mutation.prompt_cache_key.clone(),
            sticky_key: mutation.sticky_key.clone(),
            occurred_at,
            is_runtime_removed: true,
            status: "unknown".to_string(),
            is_terminal: mutation.is_terminal,
            is_success: false,
            request_tokens: 0,
            cost: 0.0,
            upstream_account_id: mutation.upstream_account_id,
            upstream_account_name: None,
            preview: None,
        })
    }
}

#[derive(Debug, Clone)]
struct ReplayableTopicEvent {
    frame: Arc<SerializedTopicFrame>,
    bytes: usize,
    emitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardTopicRevision {
    base_revision: u64,
    current_revision: Option<u64>,
    network_revision: Option<u64>,
    terminal_revision: Option<u64>,
}

#[derive(Debug)]
struct DashboardActivityMaterializerState {
    base: DashboardActivityTopicMaterializedBase,
    rebase_range_start: Option<DateTime<Utc>>,
    current_revision: Option<u64>,
    network_revision: Option<u64>,
    terminal_revision: Option<u64>,
}

impl DashboardActivityMaterializerState {
    fn new(base: DashboardActivityTopicMaterializedBase) -> Self {
        let rebase_range_start = parse_to_utc_datetime(&base.response().range_start);
        Self {
            base,
            rebase_range_start,
            current_revision: None,
            network_revision: None,
            terminal_revision: None,
        }
    }
}

#[derive(Debug)]
struct DashboardSummaryMaterializerState {
    response: StatsResponse,
    current_revision: Option<u64>,
    terminal_revision: Option<u64>,
    terminal_sequence: u64,
    range_start: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct DashboardParallelWorkMaterializerState {
    response: ParallelWorkStatsResponse,
    baseline_response: ParallelWorkStatsResponse,
    bucket_keys: BTreeMap<i64, HashSet<String>>,
    baseline_bucket_keys: BTreeMap<i64, HashSet<String>>,
    minute_keys: BTreeMap<i64, HashSet<String>>,
    baseline_minute_keys: BTreeMap<i64, HashSet<String>>,
    active_minute_stats: ParallelWorkActiveMinuteStats,
    baseline_active_minute_stats: ParallelWorkActiveMinuteStats,
    baseline_complete_minute_start_epoch: i64,
    baseline_complete_minute_end_epoch: i64,
    baseline_row_id: i64,
    range: String,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
    conversations_enabled: bool,
    baseline_identities: HashSet<String>,
    applied_identities: HashSet<String>,
    runtime_mutations: BTreeMap<String, RuntimeInvocationMutation>,
    revision: u64,
}

#[derive(Debug, Default)]
struct ParallelWorkMutationOutcome {
    changed: bool,
    needs_account_reconcile: bool,
}

impl DashboardParallelWorkMaterializerState {
    fn apply_runtime_mutation(
        &mut self,
        mutation: &RuntimeInvocationMutation,
    ) -> ParallelWorkMutationOutcome {
        let identity = format!(
            "{}\0{}",
            mutation.identity.invoke_id, mutation.identity.occurred_at
        );
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            if self.runtime_mutations.remove(&identity).is_none() {
                return ParallelWorkMutationOutcome::default();
            }
            self.applied_identities.remove(&identity);
            self.rebuild_runtime_overlay();
            return ParallelWorkMutationOutcome {
                changed: true,
                needs_account_reconcile: false,
            };
        }
        if !matches!(
            mutation.kind,
            RuntimeMutationKind::RuntimeUpsert
                | RuntimeMutationKind::TerminalCommitted
                | RuntimeMutationKind::Recovery
        ) {
            return ParallelWorkMutationOutcome::default();
        }
        if let Some(account_id) = self.upstream_account_id
            && mutation.upstream_account_id != Some(account_id)
        {
            return ParallelWorkMutationOutcome {
                changed: false,
                needs_account_reconcile: mutation.upstream_account_id.is_none()
                    && mutation.prompt_cache_key.is_some(),
            };
        }
        if self.baseline_identities.contains(&identity) {
            return ParallelWorkMutationOutcome::default();
        }
        if mutation
            .row_id
            .is_some_and(|row_id| row_id <= self.baseline_row_id)
        {
            return ParallelWorkMutationOutcome::default();
        };
        if self.applied_identities.contains(&identity) {
            return ParallelWorkMutationOutcome::default();
        }

        let changed = self.apply_runtime_overlay(mutation);
        if changed {
            self.applied_identities.insert(identity.clone());
            self.runtime_mutations.insert(identity, mutation.clone());
            self.revision = self.revision.saturating_add(1);
        }
        ParallelWorkMutationOutcome {
            changed,
            needs_account_reconcile: false,
        }
    }

    fn replay_runtime_mutations(
        &mut self,
        mutations: &BTreeMap<String, RuntimeInvocationMutation>,
    ) -> bool {
        let mut changed = false;
        for mutation in mutations.values() {
            changed |= self.apply_runtime_mutation(mutation).changed;
        }
        changed
    }

    fn apply_runtime_overlay(&mut self, mutation: &RuntimeInvocationMutation) -> bool {
        self.apply_runtime_overlay_at(mutation, Utc::now())
    }

    fn apply_runtime_overlay_at(
        &mut self,
        mutation: &RuntimeInvocationMutation,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(prompt_cache_key) = mutation.prompt_cache_key.as_ref() else {
            return false;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&mutation.identity.occurred_at) else {
            return false;
        };
        if parse_to_utc_datetime(&self.response.current.range_start)
            .is_none_or(|range_start| occurred_at < range_start)
        {
            return false;
        }

        let active_minute_stats_changed = self.refresh_active_minute_stats_at(now);

        let bucket_seconds = self.response.current.bucket_seconds;
        let Ok(bucket_start_epoch) = align_reporting_bucket_epoch(
            occurred_at.timestamp(),
            bucket_seconds,
            self.reporting_tz,
        ) else {
            return false;
        };
        let bucket_changed = self
            .bucket_keys
            .entry(bucket_start_epoch)
            .or_default()
            .insert(prompt_cache_key.clone());
        let minute_start_epoch = occurred_at.timestamp().div_euclid(60) * 60;
        let minute_keys = self.minute_keys.entry(minute_start_epoch).or_default();
        let minute_changed = minute_keys.insert(prompt_cache_key.clone());
        let minute_is_complete = minute_start_epoch >= self.baseline_complete_minute_start_epoch
            && minute_start_epoch < now.timestamp().div_euclid(60) * 60;
        if minute_changed
            && minute_is_complete
            && let Some(active_minute_count) = self.active_minute_stats.active_minute_count
        {
            let previous_minute_key_count = minute_keys.len() as i64 - 1;
            self.active_minute_stats.active_minute_count =
                Some(active_minute_count + i64::from(previous_minute_key_count == 0));
            self.active_minute_stats.parallel_count_sum += 1;
        }

        let overlay = ParallelWorkRuntimeOverlay {
            occurred_at,
            prompt_cache_key,
            bucket_start_epoch,
            bucket_changed,
            minute_changed,
            conversations_enabled: self.conversations_enabled,
            reporting_tz: self.reporting_tz,
            bucket_keys: &self.bucket_keys,
            active_minute_stats: self.active_minute_stats,
        };
        let mut changed = active_minute_stats_changed;
        for window in [
            &mut self.response.current,
            &mut self.response.minute7d,
            &mut self.response.hour30d,
            &mut self.response.day_all,
        ] {
            changed |= apply_parallel_work_runtime_overlay(window, &overlay);
        }
        changed
    }

    fn projected_active_minute_stats_at(
        &self,
        now: DateTime<Utc>,
    ) -> ParallelWorkActiveMinuteStats {
        let mut stats = self.baseline_active_minute_stats;
        let Some(mut active_minute_count) = stats.active_minute_count else {
            return stats;
        };
        let current_minute_start = now.timestamp().div_euclid(60) * 60;
        for (&minute_start_epoch, keys) in &self.minute_keys {
            if minute_start_epoch < self.baseline_complete_minute_start_epoch
                || minute_start_epoch >= current_minute_start
            {
                continue;
            }
            let baseline_keys = self.baseline_minute_keys.get(&minute_start_epoch);
            let baseline_key_count = baseline_keys.map_or(0, HashSet::len) as i64;
            let baseline_minute_was_complete =
                minute_start_epoch < self.baseline_complete_minute_end_epoch;
            if !baseline_minute_was_complete && baseline_key_count > 0 {
                active_minute_count += 1;
                stats.parallel_count_sum += baseline_key_count;
            }
            let runtime_key_count = keys
                .iter()
                .filter(|key| !baseline_keys.is_some_and(|keys| keys.contains(*key)))
                .count() as i64;
            if runtime_key_count == 0 {
                continue;
            }
            if baseline_key_count == 0 {
                active_minute_count += 1;
            }
            stats.parallel_count_sum += runtime_key_count;
        }
        stats.active_minute_count = Some(active_minute_count);
        stats
    }

    fn refresh_active_minute_stats_at(&mut self, now: DateTime<Utc>) -> bool {
        let stats = self.projected_active_minute_stats_at(now);
        if stats == self.active_minute_stats {
            return false;
        }
        self.active_minute_stats = stats;
        for window in [
            &mut self.response.current,
            &mut self.response.minute7d,
            &mut self.response.hour30d,
            &mut self.response.day_all,
        ] {
            window.active_minute_count = stats.active_minute_count;
            window.avg_count = stats.average();
        }
        true
    }

    fn rebuild_runtime_overlay(&mut self) {
        self.response = self.baseline_response.clone();
        self.bucket_keys = self.baseline_bucket_keys.clone();
        self.minute_keys = self.baseline_minute_keys.clone();
        self.active_minute_stats = self.baseline_active_minute_stats;
        self.applied_identities.clear();
        let runtime_mutations = self.runtime_mutations.clone();
        for (identity, mutation) in runtime_mutations {
            if self.apply_runtime_overlay(&mutation) {
                self.applied_identities.insert(identity);
            }
        }
        self.revision = self.revision.saturating_add(1);
    }

    fn requires_rolling_rebase(&self) -> bool {
        let Some(base_range_start) = parse_to_utc_datetime(&self.response.current.range_start)
        else {
            return true;
        };
        let Ok(current_range) = resolve_range_window(&self.range, self.reporting_tz) else {
            return true;
        };
        rolling_dashboard_window_requires_rebase(Some(base_range_start), Some(current_range.start))
            || self.projected_active_minute_stats_at(Utc::now()) != self.active_minute_stats
    }
}

#[derive(Debug)]
struct ParallelWorkRuntimeOverlay<'a> {
    occurred_at: DateTime<Utc>,
    prompt_cache_key: &'a str,
    bucket_start_epoch: i64,
    bucket_changed: bool,
    minute_changed: bool,
    conversations_enabled: bool,
    reporting_tz: Tz,
    bucket_keys: &'a BTreeMap<i64, HashSet<String>>,
    active_minute_stats: ParallelWorkActiveMinuteStats,
}

fn apply_parallel_work_runtime_overlay(
    window: &mut ParallelWorkWindowResponse,
    overlay: &ParallelWorkRuntimeOverlay<'_>,
) -> bool {
    let Some(range_start) = parse_to_utc_datetime(&window.range_start) else {
        return false;
    };
    if overlay.occurred_at < range_start {
        return false;
    }

    let mut changed = false;
    if overlay.bucket_changed {
        changed |= refresh_parallel_work_points(
            window,
            overlay.bucket_start_epoch,
            overlay.reporting_tz,
            overlay.bucket_keys,
        );
    }

    if overlay.conversations_enabled {
        let bucket_seconds = window.bucket_seconds;
        let effective_time_zone = window.effective_time_zone.clone();
        let conversation_changed = apply_parallel_work_conversation_overlay(
            &mut window.conversations,
            overlay.prompt_cache_key,
            overlay.occurred_at,
            bucket_seconds,
            effective_time_zone.as_str(),
        );
        changed |= conversation_changed;
    }

    if overlay.minute_changed {
        window.active_minute_count = overlay.active_minute_stats.active_minute_count;
        window.avg_count = overlay.active_minute_stats.average();
        changed = true;
    }
    changed
}

fn refresh_parallel_work_points(
    window: &mut ParallelWorkWindowResponse,
    bucket_start_epoch: i64,
    reporting_tz: Tz,
    bucket_keys: &BTreeMap<i64, HashSet<String>>,
) -> bool {
    let mut changed = false;
    if let Some(point) = window.points.iter_mut().find(|point| {
        parse_to_utc_datetime(&point.bucket_start)
            .is_some_and(|start| start.timestamp() == bucket_start_epoch)
    }) {
        let next_count = bucket_keys
            .get(&bucket_start_epoch)
            .map_or(0, |keys| keys.len() as i64);
        if point.parallel_count != next_count {
            point.parallel_count = next_count;
            changed = true;
        }
    } else {
        let Some(last_point) = window.points.last() else {
            return false;
        };
        let Some(mut cursor) = parse_to_utc_datetime(&last_point.bucket_end) else {
            return false;
        };
        while cursor.timestamp() <= bucket_start_epoch {
            let Ok(next_epoch) = next_reporting_bucket_epoch(
                cursor.timestamp(),
                window.bucket_seconds,
                reporting_tz,
            ) else {
                return changed;
            };
            let Some(next) = Utc.timestamp_opt(next_epoch, 0).single() else {
                return changed;
            };
            let parallel_count = bucket_keys
                .get(&cursor.timestamp())
                .map_or(0, |keys| keys.len() as i64);
            window.points.push(ParallelWorkPoint {
                bucket_start: format_utc_iso(cursor),
                bucket_end: format_utc_iso(next),
                parallel_count,
            });
            window.range_end = format_utc_iso(next);
            cursor = next;
            changed = true;
        }
    }
    if changed {
        refresh_parallel_work_point_totals(window);
    }
    changed
}

fn refresh_parallel_work_point_totals(window: &mut ParallelWorkWindowResponse) {
    let mut active_bucket_count = 0_i64;
    let mut min_count = None;
    let mut max_count = None;
    for point in &window.points {
        if point.parallel_count > 0 {
            active_bucket_count += 1;
        }
        min_count = Some(min_count.map_or(point.parallel_count, |value: i64| {
            value.min(point.parallel_count)
        }));
        max_count = Some(max_count.map_or(point.parallel_count, |value: i64| {
            value.max(point.parallel_count)
        }));
    }
    window.active_bucket_count = active_bucket_count;
    window.complete_bucket_count = window.points.len() as i64;
    window.min_count = min_count;
    window.max_count = max_count;
}

fn apply_parallel_work_conversation_overlay(
    conversations: &mut Vec<ParallelWorkConversation>,
    prompt_cache_key: &str,
    occurred_at: DateTime<Utc>,
    bucket_seconds: i64,
    effective_time_zone: &str,
) -> bool {
    let Ok(reporting_tz) = parse_reporting_tz(Some(effective_time_zone)) else {
        return false;
    };
    let Ok(start_epoch) =
        align_reporting_bucket_epoch(occurred_at.timestamp(), bucket_seconds, reporting_tz)
    else {
        return false;
    };
    let Ok(end_epoch) = next_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)
    else {
        return false;
    };
    let Some(start) = Utc.timestamp_opt(start_epoch, 0).single() else {
        return false;
    };
    let Some(end) = Utc.timestamp_opt(end_epoch, 0).single() else {
        return false;
    };

    if let Some(conversation) = conversations
        .iter_mut()
        .find(|conversation| conversation.conversation_id == prompt_cache_key)
    {
        conversation.request_count = conversation.request_count.saturating_add(1);
        if parse_to_utc_datetime(&conversation.start).is_some_and(|current| start < current) {
            conversation.start = format_utc_iso(start);
        }
        if parse_to_utc_datetime(&conversation.end).is_some_and(|current| end > current) {
            conversation.end = format_utc_iso(end);
        }
    } else {
        conversations.push(ParallelWorkConversation {
            conversation_id: prompt_cache_key.to_string(),
            start: format_utc_iso(start),
            end: format_utc_iso(end),
            request_count: 1,
        });
    }
    conversations.sort_by(|left, right| {
        right
            .end
            .cmp(&left.end)
            .then_with(|| right.request_count.cmp(&left.request_count))
            .then_with(|| left.conversation_id.cmp(&right.conversation_id))
    });
    conversations.truncate(80);
    true
}

async fn build_dashboard_parallel_work_materializer_state(
    state: &Arc<AppState>,
    query: ParallelWorkStatsQuery,
) -> Result<DashboardParallelWorkMaterializerState, ApiError> {
    for _ in 0..3 {
        let mut observer = state.pool.acquire().await?;
        let version_before = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
            .fetch_one(&mut *observer)
            .await?;
        let baseline_row_id =
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                .fetch_one(&mut *observer)
                .await?;
        let materializer = build_dashboard_parallel_work_materializer_state_at_baseline(
            state,
            query.clone(),
            baseline_row_id,
        )
        .await?;
        let version_after = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
            .fetch_one(&mut *observer)
            .await?;
        if version_before == version_after {
            return Ok(materializer);
        }
    }
    Err(ApiError::from(anyhow!(
        "parallel-work baseline changed during build"
    )))
}

async fn build_dashboard_parallel_work_materializer_state_at_baseline(
    state: &Arc<AppState>,
    query: ParallelWorkStatsQuery,
    baseline_row_id: i64,
) -> Result<DashboardParallelWorkMaterializerState, ApiError> {
    let ParallelWorkProjectionBaseline {
        response,
        bucket_keys,
        active_minute_stats,
    } = load_parallel_work_projection_baseline(state, query.clone()).await?;
    let current = &response.current;
    let range_start = parse_to_utc_datetime(&current.range_start)
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work range start")))?;
    let range_end = parse_to_utc_datetime(&current.range_end)
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work range end")))?;
    let reporting_tz = parse_reporting_tz(Some(current.effective_time_zone.as_str()))?;
    let requested_reporting_tz = parse_reporting_tz(query.time_zone.as_deref())?;
    let conversations_enabled = resolve_range_window(&query.range, requested_reporting_tz)?
        .duration
        <= ChronoDuration::hours(24);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_start_epoch = range_start.timestamp();
    let minute_keys = query_parallel_work_exact_key_sets(
        &state.pool,
        range_start,
        range_end,
        60,
        chrono_tz::UTC,
        source_scope,
        query.upstream_account_id,
        None,
        None,
    )
    .await?;
    Ok(DashboardParallelWorkMaterializerState {
        baseline_response: response.clone(),
        response,
        baseline_bucket_keys: bucket_keys.clone(),
        bucket_keys,
        baseline_minute_keys: minute_keys.clone(),
        minute_keys,
        baseline_active_minute_stats: active_minute_stats,
        active_minute_stats,
        baseline_complete_minute_start_epoch: if range_start_epoch.rem_euclid(60) == 0 {
            range_start_epoch
        } else {
            range_start_epoch.div_euclid(60) * 60 + 60
        },
        baseline_complete_minute_end_epoch: range_end.timestamp().div_euclid(60) * 60,
        baseline_row_id,
        range: query.range,
        reporting_tz,
        upstream_account_id: query.upstream_account_id,
        conversations_enabled,
        baseline_identities: HashSet::new(),
        applied_identities: HashSet::new(),
        runtime_mutations: BTreeMap::new(),
        revision: 0,
    })
}

impl DashboardSummaryMaterializerState {
    fn new(
        response: StatsResponse,
        terminal_sequence: u64,
        range_start: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            response,
            current_revision: None,
            terminal_revision: None,
            terminal_sequence,
            range_start,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkingConversationsProjectionUpdate {
    Unchanged,
    Changed,
    NeedsBoundedKeyHydration(BTreeSet<String>),
    NeedsReconcile,
}

#[derive(Debug)]
struct DashboardWorkingConversationsMaterializerState {
    response: PromptCacheConversationsResponse,
    page_size: usize,
    recent_invocation_limit: usize,
    blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    baseline_has_more: bool,
}

impl DashboardWorkingConversationsMaterializerState {
    fn new(
        response: PromptCacheConversationsResponse,
        page_size: i64,
        recent_invocation_limit: i64,
        blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    ) -> Self {
        let baseline_has_more = response.has_more;
        Self {
            response,
            page_size: page_size.max(0) as usize,
            recent_invocation_limit: recent_invocation_limit.max(0) as usize,
            blocked_binding_filter,
            baseline_has_more,
        }
    }

    fn apply_deltas(
        &mut self,
        records: &[PromptCacheTopicDelta],
        applied_terminal_ids: &mut HashSet<String>,
        baseline_row_id: i64,
    ) -> Result<WorkingConversationsProjectionUpdate, ApiError> {
        let now = Utc::now();
        let mut bounded_hydration_keys = BTreeSet::new();
        let mut reconcile_required = false;
        for record in records {
            let Some(prompt_cache_key) = record.prompt_cache_key.as_deref() else {
                continue;
            };
            if record.is_runtime_removed {
                continue;
            }
            let conversation = self
                .response
                .conversations
                .iter()
                .find(|conversation| conversation.prompt_cache_key == prompt_cache_key);
            let conversation_is_visible = conversation.is_some();
            let Some(preview) = record.preview.as_ref() else {
                reconcile_required = true;
                continue;
            };
            if !self.matches_blocked_binding_filter(record, preview)
                || !Self::is_in_working_window(record, now)
            {
                if conversation_is_visible {
                    bounded_hydration_keys.insert(prompt_cache_key.to_string());
                }
                continue;
            }
            if !conversation_is_visible {
                // The compact delta carries only the changing invocation. Hydration owns all
                // historical totals, charts, account and owner metadata for a newly selected
                // key before it can be applied to the typed projection.
                bounded_hydration_keys.insert(prompt_cache_key.to_string());
            } else if conversation.is_some_and(|conversation| {
                Self::delta_requires_account_hydration(
                    conversation,
                    record,
                    applied_terminal_ids,
                    baseline_row_id,
                )
            }) {
                // The wire response retains only the top account summaries. Once it is full,
                // a newly observed account may have historical totals that were omitted from
                // the response, so only the bounded key hydrate can update it exactly.
                bounded_hydration_keys.insert(prompt_cache_key.to_string());
            }
        }
        if reconcile_required {
            return Ok(WorkingConversationsProjectionUpdate::NeedsReconcile);
        }
        if !bounded_hydration_keys.is_empty() {
            return Ok(
                WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(
                    bounded_hydration_keys,
                ),
            );
        }

        let mut changed = false;

        for record in records {
            let Some(prompt_cache_key) = record.prompt_cache_key.as_deref() else {
                continue;
            };
            let conversation_index = self
                .response
                .conversations
                .iter()
                .position(|conversation| conversation.prompt_cache_key == prompt_cache_key);

            if record.is_runtime_removed {
                if let Some(index) = conversation_index {
                    changed |= self.remove_runtime_preview(index, record);
                }
                continue;
            }

            let Some(preview) = record.preview.as_ref() else {
                unreachable!("bounded hydration preflight must handle incomplete deltas");
            };
            if !self.matches_blocked_binding_filter(record, preview)
                || !Self::is_in_working_window(record, now)
            {
                continue;
            }
            let conversation_index = conversation_index.expect(
                "bounded hydration preflight must hydrate a missing working conversation key",
            );

            let conversation = &mut self.response.conversations[conversation_index];
            changed |= apply_working_conversation_delta(
                conversation,
                record,
                preview,
                self.recent_invocation_limit,
                applied_terminal_ids,
                baseline_row_id,
            );
        }

        if changed {
            self.refresh_pagination();
        }

        Ok(if changed {
            WorkingConversationsProjectionUpdate::Changed
        } else {
            WorkingConversationsProjectionUpdate::Unchanged
        })
    }

    fn apply_binding(
        &mut self,
        prompt_cache_key: &str,
        binding: &PromptCacheConversationBindingResponse,
    ) -> Option<bool> {
        let conversation = self
            .response
            .conversations
            .iter_mut()
            .find(|conversation| conversation.prompt_cache_key == prompt_cache_key)?;
        let manual_binding = (binding.binding_kind != "none").then(|| {
            PromptCacheConversationManualBindingResponse {
                binding_kind: binding.binding_kind.clone(),
                group_name: binding.group_name.clone(),
                upstream_account_id: binding.upstream_account_id,
                upstream_account_name: binding.upstream_account_name.clone(),
            }
        });
        let changed = conversation.has_encrypted_session_owner
            != binding.has_encrypted_session_owner
            || conversation.encrypted_owner_account_id != binding.encrypted_owner_account_id
            || conversation.encrypted_owner_account_name != binding.encrypted_owner_account_name
            || conversation.encrypted_owner_group_name != binding.encrypted_owner_group_name
            || conversation.manual_binding != manual_binding;
        if changed {
            conversation.has_encrypted_session_owner = binding.has_encrypted_session_owner;
            conversation.encrypted_owner_account_id = binding.encrypted_owner_account_id;
            conversation.encrypted_owner_account_name =
                binding.encrypted_owner_account_name.clone();
            conversation.encrypted_owner_group_name = binding.encrypted_owner_group_name.clone();
            conversation.manual_binding = manual_binding;
        }
        Some(changed)
    }

    fn serialize(&self) -> Result<Vec<u8>, ApiError> {
        serde_json::to_vec(&self.response).map_err(ApiError::from)
    }

    fn matches_blocked_binding_filter(
        &self,
        record: &PromptCacheTopicDelta,
        preview: &PromptCacheConversationInvocationPreviewResponse,
    ) -> bool {
        let Some(filter) = self.blocked_binding_filter.as_ref() else {
            return true;
        };
        if !filter.is_active() || !record.is_terminal {
            return !filter.is_active();
        }
        let Some(blocked_binding) = preview.blocked_binding.as_ref() else {
            return false;
        };
        filter
            .upstream_account_id
            .is_none_or(|account_id| account_id == blocked_binding.upstream_account_id)
            && filter
                .constraint_source
                .is_none_or(|source| source == blocked_binding.constraint_source)
    }

    fn is_in_working_window(record: &PromptCacheTopicDelta, now: DateTime<Utc>) -> bool {
        !record.is_terminal
            || parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
                occurred_at
                    >= now
                        - ChronoDuration::minutes(
                            SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                        )
            })
    }

    fn delta_requires_account_hydration(
        conversation: &PromptCacheConversationResponse,
        record: &PromptCacheTopicDelta,
        applied_terminal_ids: &HashSet<String>,
        baseline_row_id: i64,
    ) -> bool {
        let already_terminal = applied_terminal_ids.contains(&record.identity)
            || (record.row_id > 0 && record.row_id <= baseline_row_id);
        if !record.is_terminal
            || already_terminal
            || conversation.upstream_accounts.len()
                < PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT
        {
            return false;
        }
        let account_group_key = resolve_prompt_cache_upstream_account_group_key(
            record.upstream_account_id,
            record.upstream_account_name.as_deref(),
        );
        !conversation.upstream_accounts.iter().any(|account| {
            resolve_prompt_cache_upstream_account_group_key(
                account.upstream_account_id,
                account.upstream_account_name.as_deref(),
            ) == account_group_key
        })
    }

    fn delta_is_eligible(&self, record: &PromptCacheTopicDelta, now: DateTime<Utc>) -> bool {
        !record.is_runtime_removed
            && record.preview.as_ref().is_some_and(|preview| {
                self.matches_blocked_binding_filter(record, preview)
                    && Self::is_in_working_window(record, now)
            })
    }

    fn replace_hydrated_conversation(
        &mut self,
        prompt_cache_key: &str,
        hydrated: Option<PromptCacheConversationResponse>,
    ) -> bool {
        let existing_index = self
            .response
            .conversations
            .iter()
            .position(|conversation| conversation.prompt_cache_key == prompt_cache_key);
        let changed = match (existing_index, hydrated) {
            (Some(index), Some(hydrated)) => {
                if self.response.conversations[index] == hydrated {
                    false
                } else {
                    self.response.conversations[index] = hydrated;
                    true
                }
            }
            (Some(index), None) => {
                self.response.conversations.remove(index);
                true
            }
            (None, Some(hydrated)) => {
                self.response.conversations.push(hydrated);
                true
            }
            (None, None) => false,
        };
        if changed {
            self.refresh_pagination();
        }
        changed
    }

    fn set_total_matched(&mut self, total_matched: i64) -> bool {
        if self.response.total_matched == Some(total_matched) {
            return false;
        }
        self.response.total_matched = Some(total_matched);
        self.refresh_pagination();
        true
    }

    fn visible_keys(&self) -> HashSet<String> {
        self.response
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.clone())
            .collect()
    }

    fn refresh_pagination(&mut self) {
        // The cold baseline owns continuation cursor semantics. Live deltas may reorder the
        // active display page, but they never manufacture a cursor for that changed ordering:
        // such a cursor would be applied to the original database snapshot and skip or repeat
        // rows. Fresh page-size subscriptions establish a new baseline when needed.
        sort_working_conversation_responses(&mut self.response.conversations);
        let overflowed = self.response.conversations.len() > self.page_size;
        self.response.conversations.truncate(self.page_size);
        self.response.has_more = self.baseline_has_more
            || overflowed
            || self.response.total_matched.is_some_and(|total_matched| {
                total_matched > self.response.conversations.len() as i64
            });
        for conversation in &mut self.response.conversations {
            conversation.cursor = None;
        }
        self.response.next_cursor = None;
    }

    fn remove_runtime_preview(
        &mut self,
        conversation_index: usize,
        record: &PromptCacheTopicDelta,
    ) -> bool {
        let conversation = &mut self.response.conversations[conversation_index];
        let before = conversation.recent_invocations.len();
        conversation.recent_invocations.retain(|preview| {
            preview.invoke_id != record.invoke_id
                || !working_timestamp_cmp(&preview.occurred_at, &record.occurred_at).is_eq()
        });
        if conversation.recent_invocations.len() == before {
            return false;
        }

        conversation.last_in_flight_at = working_conversation_latest_in_flight_at(conversation);
        conversation.blocked_binding = conversation
            .recent_invocations
            .iter()
            .find_map(|preview| preview.blocked_binding.clone());
        if let Some(last_activity_at) = working_conversation_latest_activity_at(conversation) {
            conversation.last_activity_at = last_activity_at;
        }

        if conversation.request_count == 0
            && let Some(first_preview) = conversation.recent_invocations.last()
        {
            conversation.created_at = first_preview.occurred_at.clone();
        }

        if conversation.request_count == 0 && conversation.recent_invocations.is_empty() {
            self.response.conversations.remove(conversation_index);
            if let Some(total_matched) = self.response.total_matched.as_mut() {
                *total_matched = total_matched.saturating_sub(1);
            }
        }
        true
    }

    fn expire(&mut self, now: DateTime<Utc>) -> bool {
        let activity_cutoff = now
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES);
        let request_cutoff = now - ChronoDuration::hours(24);
        let before = self.response.conversations.len();
        self.response.conversations.retain(|conversation| {
            conversation.last_in_flight_at.is_some()
                || parse_to_utc_datetime(&conversation.last_activity_at)
                    .is_some_and(|last_activity_at| last_activity_at >= activity_cutoff)
        });
        let removed = before.saturating_sub(self.response.conversations.len());
        if removed > 0
            && let Some(total_matched) = self.response.total_matched.as_mut()
        {
            *total_matched = total_matched.saturating_sub(removed as i64);
        }
        let mut changed = removed > 0;
        for conversation in &mut self.response.conversations {
            let before = conversation.last24h_requests.len();
            conversation.last24h_requests.retain(|point| {
                parse_to_utc_datetime(&point.occurred_at)
                    .is_some_and(|occurred_at| occurred_at >= request_cutoff)
            });
            changed |= conversation.last24h_requests.len() != before;
            let mut cumulative_tokens = 0_i64;
            for point in &mut conversation.last24h_requests {
                cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
                point.cumulative_tokens = cumulative_tokens;
            }
        }
        if changed {
            self.refresh_pagination();
        }
        changed
    }
}

fn apply_working_conversation_delta(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
    recent_invocation_limit: usize,
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) -> bool {
    let mut changed = false;
    if let Some(index) = conversation.recent_invocations.iter().position(|existing| {
        existing.invoke_id == preview.invoke_id && existing.occurred_at == preview.occurred_at
    }) {
        if conversation.recent_invocations[index] != *preview {
            conversation.recent_invocations[index] = preview.clone();
            changed = true;
        }
    } else {
        conversation.recent_invocations.push(preview.clone());
        changed = true;
    }
    conversation.recent_invocations.sort_by(|left, right| {
        working_timestamp_cmp(&right.occurred_at, &left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    if conversation.recent_invocations.len() > recent_invocation_limit {
        conversation
            .recent_invocations
            .truncate(recent_invocation_limit);
    }

    changed |= update_string_if_newer(&mut conversation.last_activity_at, &record.occurred_at);
    changed |= update_string_if_earlier(&mut conversation.created_at, &record.occurred_at);

    let already_terminal = applied_terminal_ids.contains(&record.identity)
        || (record.row_id > 0 && record.row_id <= baseline_row_id);
    if record.is_terminal && !already_terminal {
        applied_terminal_ids.insert(record.identity.clone());
        apply_working_conversation_terminal_delta(conversation, record, preview);
        changed = true;
    }
    if !record.is_terminal {
        changed |= update_option_if_newer(&mut conversation.last_in_flight_at, &record.occurred_at);
    } else {
        let latest_in_flight = working_conversation_latest_in_flight_at(conversation);
        if conversation.last_in_flight_at != latest_in_flight {
            conversation.last_in_flight_at = latest_in_flight;
            changed = true;
        }
    }

    let blocked_binding = conversation
        .recent_invocations
        .iter()
        .find_map(|candidate| candidate.blocked_binding.clone());
    if conversation.blocked_binding != blocked_binding {
        conversation.blocked_binding = blocked_binding;
        changed = true;
    }
    changed
}

fn working_conversation_latest_in_flight_at(
    conversation: &PromptCacheConversationResponse,
) -> Option<String> {
    conversation
        .recent_invocations
        .iter()
        .filter(|preview| {
            !prompt_invocation_status_counts_toward_terminal_totals(Some(&preview.status))
        })
        .map(|preview| preview.occurred_at.clone())
        .max_by(|left, right| working_timestamp_cmp(left, right))
}

fn working_conversation_latest_activity_at(
    conversation: &PromptCacheConversationResponse,
) -> Option<String> {
    conversation.recent_invocations.iter().fold(
        conversation.last_terminal_at.clone(),
        |latest, preview| match latest {
            Some(latest) if !working_timestamp_cmp(&preview.occurred_at, &latest).is_gt() => {
                Some(latest)
            }
            _ => Some(preview.occurred_at.clone()),
        },
    )
}

fn apply_working_conversation_terminal_delta(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
) {
    conversation.request_count = conversation.request_count.saturating_add(1);
    conversation.total_tokens = conversation
        .total_tokens
        .saturating_add(record.request_tokens.max(0));
    conversation.total_cost += record.cost;
    update_option_if_newer(&mut conversation.last_terminal_at, &record.occurred_at);

    let account_group_key = resolve_prompt_cache_upstream_account_group_key(
        record.upstream_account_id,
        record.upstream_account_name.as_deref(),
    );
    let account_index = conversation
        .upstream_accounts
        .iter()
        .position(|account| {
            resolve_prompt_cache_upstream_account_group_key(
                account.upstream_account_id,
                account.upstream_account_name.as_deref(),
            ) == account_group_key
        })
        .unwrap_or_else(|| {
            conversation
                .upstream_accounts
                .push(PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: record.occurred_at.clone(),
                });
            conversation.upstream_accounts.len() - 1
        });
    let account = &mut conversation.upstream_accounts[account_index];
    if account.upstream_account_id.is_none() && record.upstream_account_id.is_some() {
        account.upstream_account_id = record.upstream_account_id;
    }
    if account.upstream_account_name.is_none() && record.upstream_account_name.is_some() {
        account.upstream_account_name = record.upstream_account_name.clone();
    }
    account.request_count = account.request_count.saturating_add(1);
    account.total_tokens = account
        .total_tokens
        .saturating_add(record.request_tokens.max(0));
    account.total_cost += record.cost;
    update_string_if_newer(&mut account.last_activity_at, &record.occurred_at);
    conversation.upstream_accounts.sort_by(|left, right| {
        working_timestamp_cmp(&right.last_activity_at, &left.last_activity_at)
            .then_with(|| {
                resolve_prompt_cache_upstream_account_label(
                    right.upstream_account_name.as_deref(),
                    right.upstream_account_id,
                )
                .cmp(&resolve_prompt_cache_upstream_account_label(
                    left.upstream_account_name.as_deref(),
                    left.upstream_account_id,
                ))
            })
            .then_with(|| {
                right
                    .upstream_account_id
                    .unwrap_or(i64::MIN)
                    .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
            })
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| right.request_count.cmp(&left.request_count))
    });
    conversation
        .upstream_accounts
        .truncate(PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT);

    let outcome = invocation_point_outcome(
        Some(&record.status),
        preview.error_message.as_deref(),
        preview.downstream_error_message.as_deref(),
        preview.failure_kind.as_deref(),
        preview.failure_class.as_deref(),
    )
    .to_string();
    conversation
        .last24h_requests
        .push(PromptCacheConversationRequestPointResponse {
            occurred_at: record.occurred_at.clone(),
            status: if record.status.trim().is_empty() {
                "unknown".to_string()
            } else {
                record.status.clone()
            },
            is_success: outcome == "success",
            outcome,
            request_tokens: record.request_tokens.max(0),
            cumulative_tokens: 0,
        });
    conversation
        .last24h_requests
        .sort_by(|left, right| working_timestamp_cmp(&left.occurred_at, &right.occurred_at));
    let mut cumulative_tokens = 0_i64;
    for point in &mut conversation.last24h_requests {
        cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
        point.cumulative_tokens = cumulative_tokens;
    }
}

fn sort_working_conversation_responses(conversations: &mut [PromptCacheConversationResponse]) {
    conversations.sort_by(|left, right| {
        working_timestamp_cmp(
            working_conversation_sort_anchor(right),
            working_conversation_sort_anchor(left),
        )
        .then_with(|| working_timestamp_cmp(&right.created_at, &left.created_at))
        .then_with(|| right.prompt_cache_key.cmp(&left.prompt_cache_key))
    });
}

fn working_conversation_sort_anchor(conversation: &PromptCacheConversationResponse) -> &str {
    resolve_working_conversation_sort_anchor(
        conversation.last_terminal_at.as_deref(),
        conversation.last_in_flight_at.as_deref(),
        &conversation.created_at,
    )
}

fn working_timestamp_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    match (parse_to_utc_datetime(left), parse_to_utc_datetime(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn update_string_if_newer(current: &mut String, candidate: &str) -> bool {
    if working_timestamp_cmp(candidate, current).is_gt() {
        *current = candidate.to_string();
        true
    } else {
        false
    }
}

fn update_string_if_earlier(current: &mut String, candidate: &str) -> bool {
    if working_timestamp_cmp(candidate, current).is_lt() {
        *current = candidate.to_string();
        true
    } else {
        false
    }
}

fn update_option_if_newer(current: &mut Option<String>, candidate: &str) -> bool {
    if current
        .as_deref()
        .is_none_or(|existing| working_timestamp_cmp(candidate, existing).is_gt())
    {
        *current = Some(candidate.to_string());
        true
    } else {
        false
    }
}

#[derive(Debug, Clone)]
enum DashboardTopicMaterializer {
    Activity {
        base: Arc<StdMutex<DashboardActivityMaterializerState>>,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
    },
    Summary {
        base: Arc<StdMutex<DashboardSummaryMaterializerState>>,
        window: SummaryWindow,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
        upstream_account_id: Option<i64>,
    },
    NetworkTimeseries {
        base: Arc<DashboardNetworkTimeseriesResponse>,
        upstream_account_id: Option<i64>,
    },
    NetworkRecent {
        base: Arc<DashboardRecentNetworkWindowResponse>,
    },
    Timeseries {
        base: Arc<StdMutex<TimeseriesTopicMaterializedBase>>,
        runtime: Arc<RuntimeProjectionHub>,
    },
    ParallelWork {
        base: Arc<StdMutex<DashboardParallelWorkMaterializerState>>,
    },
    WorkingConversations {
        state: Arc<StdMutex<DashboardWorkingConversationsMaterializerState>>,
    },
}

impl DashboardTopicMaterializer {
    fn revision(
        &self,
        base_revision: u64,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Option<DashboardTopicRevision> {
        match self {
            Self::Activity { .. }
                if current.is_some() || network.is_some() || terminal.is_some() =>
            {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: network.map(|slice| slice.revision),
                    terminal_revision: terminal.map(|slice| slice.revision),
                })
            }
            Self::Summary { .. } if current.is_some() || terminal.is_some() => {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: None,
                    terminal_revision: terminal.map(|slice| slice.revision),
                })
            }
            Self::NetworkTimeseries {
                base,
                upstream_account_id,
            } => network
                .filter(|slice| {
                    dashboard_network_timeseries_live_point(
                        base.as_ref(),
                        *upstream_account_id,
                        Some(slice),
                    )
                    .is_some()
                })
                .map(|slice| DashboardTopicRevision {
                    base_revision,
                    current_revision: None,
                    network_revision: Some(slice.revision),
                    terminal_revision: None,
                }),
            Self::NetworkRecent { .. } => network.map(|slice| DashboardTopicRevision {
                base_revision,
                current_revision: None,
                network_revision: Some(slice.revision),
                terminal_revision: None,
            }),
            Self::Timeseries { .. } if current.is_some() || terminal.is_some() => {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: None,
                    terminal_revision: terminal.map(|slice| slice.revision),
                })
            }
            Self::ParallelWork { base } => Some(DashboardTopicRevision {
                base_revision,
                current_revision: Some(
                    base.lock()
                        .expect("parallel-work materializer state lock")
                        .revision,
                ),
                network_revision: None,
                terminal_revision: None,
            }),
            Self::WorkingConversations { .. } => None,
            _ => None,
        }
    }

    fn requires_terminal_window_rebase(&self) -> bool {
        match self {
            Self::Activity {
                base, reporting_tz, ..
            } => {
                let base = base.lock().expect("activity materializer state lock");
                let Ok(range) = resolve_dashboard_activity_cached_range(
                    &base.base.response().range,
                    *reporting_tz,
                ) else {
                    return true;
                };
                if parse_duration_spec(&base.base.response().range).is_ok() {
                    return rolling_dashboard_window_requires_rebase(
                        base.rebase_range_start,
                        Some(range.start),
                    );
                }
                base.rebase_range_start != Some(range.start)
            }
            Self::Summary {
                base,
                window,
                reporting_tz,
                ..
            } => {
                let base = base.lock().expect("summary materializer state lock");
                let current_range_start = summary_window_range(window, *reporting_tz, Utc::now())
                    .ok()
                    .flatten()
                    .map(|(start, _)| start);
                match (window, base.range_start, current_range_start) {
                    (SummaryWindow::Duration(_), base_start, current_start) => {
                        rolling_dashboard_window_requires_rebase(base_start, current_start)
                    }
                    (_, base_start, current_start) => base_start != current_start,
                }
            }
            Self::Timeseries { base, .. } => base
                .lock()
                .expect("timeseries materializer state lock")
                .requires_window_rebase(),
            Self::ParallelWork { base } => base
                .lock()
                .expect("parallel-work materializer state lock")
                .requires_rolling_rebase(),
            Self::NetworkTimeseries { .. }
            | Self::NetworkRecent { .. }
            | Self::WorkingConversations { .. } => false,
        }
    }

    fn serialize(
        &self,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Result<Vec<u8>, ApiError> {
        match self {
            Self::Activity {
                base,
                reporting_tz,
                source_scope,
            } => {
                let mut base = base.lock().expect("activity materializer state lock");
                if current.is_some_and(|slice| base.current_revision < Some(slice.revision)) {
                    apply_dashboard_activity_slices(base.base.response_mut(), current, None);
                    base.current_revision = current.map(|slice| slice.revision);
                }
                if network.is_some_and(|slice| base.network_revision < Some(slice.revision)) {
                    apply_dashboard_activity_slices(base.base.response_mut(), None, network);
                    base.network_revision = network.map(|slice| slice.revision);
                }
                if terminal.is_some_and(|slice| base.terminal_revision < Some(slice.revision)) {
                    base.base.apply_terminal_slice(
                        *reporting_tz,
                        *source_scope,
                        terminal.expect("terminal slice checked above"),
                    );
                    base.terminal_revision = terminal.map(|slice| slice.revision);
                }
                serde_json::to_vec(base.base.response()).map_err(ApiError::from)
            }
            Self::Summary {
                base,
                window,
                reporting_tz,
                source_scope,
                upstream_account_id,
            } => {
                let mut base = base.lock().expect("summary materializer state lock");
                if current.is_some_and(|slice| base.current_revision < Some(slice.revision)) {
                    apply_dashboard_current_slice_to_summary_response(
                        &mut base.response,
                        *upstream_account_id,
                        current,
                    );
                    base.current_revision = current.map(|slice| slice.revision);
                }
                if terminal.is_some_and(|slice| base.terminal_revision < Some(slice.revision)) {
                    let DashboardSummaryMaterializerState {
                        response,
                        terminal_sequence,
                        ..
                    } = &mut *base;
                    apply_dashboard_terminal_slice_to_summary_response(
                        response,
                        terminal_sequence,
                        window,
                        *reporting_tz,
                        *source_scope,
                        *upstream_account_id,
                        terminal.expect("terminal slice checked above"),
                    );
                    base.terminal_revision = terminal.map(|slice| slice.revision);
                }
                serde_json::to_vec(&base.response).map_err(ApiError::from)
            }
            Self::NetworkTimeseries {
                base,
                upstream_account_id,
            } => serde_json::to_vec(&DashboardNetworkTimeseriesPayload {
                base,
                upstream_account_id: *upstream_account_id,
                network,
            })
            .map_err(ApiError::from),
            Self::NetworkRecent { base } => {
                serde_json::to_vec(&DashboardNetworkRecentPayload { base, network })
                    .map_err(ApiError::from)
            }
            Self::Timeseries { base, runtime } => {
                let mut base = base.lock().expect("timeseries materializer state lock");
                base.apply_terminal_slice(terminal);
                base.serialize(&runtime.snapshot())
            }
            Self::ParallelWork { base } => serde_json::to_vec(
                &base
                    .lock()
                    .expect("parallel-work materializer state lock")
                    .response,
            )
            .map_err(ApiError::from),
            Self::WorkingConversations { state } => state
                .lock()
                .expect("working conversations materializer state lock")
                .serialize(),
        }
    }
}

struct DashboardNetworkTimeseriesPayload<'a> {
    base: &'a DashboardNetworkTimeseriesResponse,
    upstream_account_id: Option<i64>,
    network: Option<&'a DashboardNetworkProjectionSlice>,
}

impl Serialize for DashboardNetworkTimeseriesPayload<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let live_point = dashboard_network_timeseries_live_point(
            self.base,
            self.upstream_account_id,
            self.network,
        );
        let mut response = serializer.serialize_struct("DashboardNetworkTimeseriesResponse", 6)?;
        response.serialize_field("range", &self.base.range)?;
        response.serialize_field("rangeStart", &self.base.range_start)?;
        if let Some((_, _)) = live_point {
            let now = Utc::now();
            response.serialize_field("rangeEnd", &format_utc_iso_precise(now))?;
            response.serialize_field("snapshotId", &now.timestamp_millis())?;
        } else {
            response.serialize_field("rangeEnd", &self.base.range_end)?;
            response.serialize_field("snapshotId", &self.base.snapshot_id)?;
        }
        response.serialize_field("bucketSeconds", &self.base.bucket_seconds)?;
        response.serialize_field(
            "points",
            &DashboardNetworkTimeseriesPoints {
                points: &self.base.points,
                live_point,
            },
        )?;
        response.end()
    }
}

struct DashboardNetworkTimeseriesPoints<'a> {
    points: &'a [DashboardNetworkTimeseriesPointResponse],
    live_point: Option<(usize, &'a DashboardNetworkTimeseriesPointResponse)>,
}

impl Serialize for DashboardNetworkTimeseriesPoints<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut points = serializer.serialize_seq(Some(self.points.len()))?;
        for (index, point) in self.points.iter().enumerate() {
            let point = self
                .live_point
                .filter(|(live_index, _)| *live_index == index)
                .map_or(point, |(_, live_point)| live_point);
            points.serialize_element(point)?;
        }
        points.end()
    }
}

struct DashboardNetworkRecentPayload<'a> {
    base: &'a DashboardRecentNetworkWindowResponse,
    network: Option<&'a DashboardNetworkProjectionSlice>,
}

impl Serialize for DashboardNetworkRecentPayload<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.network
            .map_or(self.base, |slice| &slice.recent)
            .serialize(serializer)
    }
}

#[derive(Debug, Clone)]
enum BuiltSubscriptionTopicPayload {
    Json(Value),
    Dashboard(DashboardTopicMaterializer),
}

impl BuiltSubscriptionTopicPayload {
    fn dashboard_materializer(&self) -> Option<DashboardTopicMaterializer> {
        match self {
            Self::Json(_) => None,
            Self::Dashboard(materializer) => Some(materializer.clone()),
        }
    }

    fn serialize(
        &self,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Result<Vec<u8>, ApiError> {
        match self {
            Self::Json(payload) => serde_json::to_vec(payload).map_err(ApiError::from),
            Self::Dashboard(materializer) => materializer.serialize(current, network, terminal),
        }
    }

    fn snapshot_payload(&self) -> Value {
        match self {
            Self::Json(payload) => payload.clone(),
            Self::Dashboard(_) => Value::Null,
        }
    }
}

#[derive(Debug, Clone)]
struct PendingDashboardTopicMaterialization {
    topic_key: String,
    topic_name: &'static str,
    revision: DashboardTopicRevision,
    materializer: DashboardTopicMaterializer,
}

struct ServerPushTopicLease {
    hub: Arc<SubscriptionHub>,
    topic_keys: Vec<String>,
}

pub(crate) struct TopicSubscriptionLease {
    hub: Arc<SubscriptionHub>,
    topic_keys: Vec<String>,
    topic_names: Vec<String>,
    owns_dashboard_live: bool,
}

impl Drop for TopicSubscriptionLease {
    fn drop(&mut self) {
        if self.topic_keys.is_empty() {
            return;
        }
        let hub = self.hub.clone();
        let topic_keys = std::mem::take(&mut self.topic_keys);
        let topic_names = std::mem::take(&mut self.topic_names);
        let owns_dashboard_live = self.owns_dashboard_live;
        tokio::spawn(async move {
            hub.release_topic_subscribers(topic_keys, topic_names, owns_dashboard_live)
                .await;
        });
    }
}

impl Drop for ServerPushTopicLease {
    fn drop(&mut self) {
        if self.topic_keys.is_empty() {
            return;
        }
        let hub = self.hub.clone();
        let topic_keys = std::mem::take(&mut self.topic_keys);
        tokio::spawn(async move {
            hub.release_server_push_topics(topic_keys).await;
        });
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ReplayMissReason {
    SchemaEpochMismatch,
    GapWindowMiss,
    GapEventBudgetExceeded,
    GapByteBudgetExceeded,
    UnknownTopic,
    ContinuityReset,
}

impl ReplayMissReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SchemaEpochMismatch => "schema_epoch_mismatch",
            Self::GapWindowMiss => "gap_window_miss",
            Self::GapEventBudgetExceeded => "gap_event_budget_exceeded",
            Self::GapByteBudgetExceeded => "gap_byte_budget_exceeded",
            Self::UnknownTopic => "unknown_topic",
            Self::ContinuityReset => "continuity_reset",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TopicInitDisposition {
    ReplayHit,
    ResumeCaughtUp,
    SnapshotNoResume,
    SnapshotResumeMiss,
}

impl TopicInitDisposition {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ReplayHit => "replay_hit",
            Self::ResumeCaughtUp => "resume_caught_up",
            Self::SnapshotNoResume => "snapshot_no_resume",
            Self::SnapshotResumeMiss => "snapshot_resume_miss",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TopicInitOutcome {
    pub(crate) topic_key: String,
    pub(crate) disposition: TopicInitDisposition,
    pub(crate) replay_event_count: usize,
    pub(crate) replay_bytes: usize,
    pub(crate) cursor: u64,
    pub(crate) miss_reason: Option<&'static str>,
}

#[derive(Debug)]
pub(crate) struct PreparedSubscriptionConnection {
    pub(crate) initial: Vec<PreparedTopicFrame>,
    pub(crate) last_sent_cursors: HashMap<String, u64>,
    pub(crate) outcomes: Vec<TopicInitOutcome>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedTopicFrame {
    pub(crate) frame: Arc<SerializedTopicFrame>,
    kind: TopicFrameKind,
}

#[derive(Debug, Clone)]
enum SubscriptionTopic {
    AppVersion,
    QuotaCurrent,
    DashboardActivityCurrent {
        range: String,
        time_zone: String,
        recent_limit: i64,
        include_accounts: bool,
        include_recent: bool,
    },
    DashboardNetworkTimeseriesWindow {
        range: String,
        time_zone: String,
        upstream_account_id: Option<i64>,
    },
    DashboardNetworkRecentCurrent,
    DashboardWorkingConversationsCurrent {
        page_size: i64,
        recent_invocation_limit: i64,
        blocked_binding_upstream_account_id: Option<i64>,
        blocked_binding_constraint_source: Option<BlockedBindingConstraintSource>,
    },
    InvocationWindow {
        limit: i64,
        model: Option<String>,
        status: Option<String>,
    },
    InvocationHistoryWindow {
        scope: ConversationSubscriptionScope,
    },
    InvocationHistoryOverview {
        scope: ConversationSubscriptionScope,
    },
    PromptCacheConversationBindingCurrent {
        scope: ConversationSubscriptionScope,
    },
    PromptCacheConversationOperationsWindow {
        scope: ConversationSubscriptionScope,
        info_type: Option<String>,
    },
    PromptCacheWindow {
        selection: PromptCacheConversationSelection,
        detail_level: PromptCacheConversationDetailLevel,
        recent_invocation_limit: Option<i64>,
    },
    PromptCacheStickyWindow {
        account_id: i64,
        selection: AccountStickyKeySelection,
    },
    SummaryCurrent {
        window: String,
        time_zone: String,
        limit: Option<i64>,
        upstream_account_id: Option<i64>,
    },
    TimeseriesOpenWindow {
        range: String,
        time_zone: String,
        bucket: Option<String>,
        settlement_hour: Option<u8>,
        upstream_account_id: Option<i64>,
    },
    ParallelWorkCurrent {
        range: String,
        time_zone: String,
        bucket: Option<String>,
        upstream_account_id: Option<i64>,
    },
    ForwardProxyLive,
    InvocationPoolAttempts {
        invoke_id: String,
    },
    ModelRoutingLive {
        window: String,
        model: Option<String>,
        state: Option<String>,
        limit: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConversationSubscriptionScope {
    PromptCacheKey(String),
    StickyKey {
        sticky_key: String,
        upstream_account_id: i64,
    },
}

impl ConversationSubscriptionScope {
    fn binding_key(&self) -> &str {
        match self {
            Self::PromptCacheKey(prompt_cache_key) => prompt_cache_key,
            Self::StickyKey { sticky_key, .. } => sticky_key,
        }
    }

    fn matches_record(&self, record: &ApiInvocation) -> bool {
        match self {
            Self::PromptCacheKey(prompt_cache_key) => record
                .prompt_cache_key
                .as_deref()
                .is_some_and(|value| value.trim() == prompt_cache_key),
            Self::StickyKey {
                sticky_key,
                upstream_account_id,
            } => {
                record
                    .sticky_key
                    .as_deref()
                    .or(record.prompt_cache_key.as_deref())
                    .is_some_and(|value| value.trim() == sticky_key)
                    && record.upstream_account_id == Some(*upstream_account_id)
            }
        }
    }

    fn matches_runtime_mutation(&self, mutation: &RuntimeInvocationMutation) -> bool {
        match self {
            Self::PromptCacheKey(prompt_cache_key) => mutation
                .prompt_cache_key
                .as_deref()
                .is_some_and(|value| value == prompt_cache_key),
            Self::StickyKey {
                sticky_key,
                upstream_account_id,
            } => {
                mutation
                    .sticky_key
                    .as_deref()
                    .or(mutation.prompt_cache_key.as_deref())
                    .is_some_and(|value| value == sticky_key)
                    && mutation.upstream_account_id == Some(*upstream_account_id)
            }
        }
    }

    fn matches_sticky_route_change(
        &self,
        sticky_key: &str,
        previous_upstream_account_id: i64,
        upstream_account_id: i64,
    ) -> bool {
        match self {
            Self::PromptCacheKey(prompt_cache_key) => prompt_cache_key == sticky_key,
            Self::StickyKey {
                sticky_key: current_sticky_key,
                upstream_account_id: current_upstream_account_id,
            } => {
                current_sticky_key == sticky_key
                    && (*current_upstream_account_id == previous_upstream_account_id
                        || *current_upstream_account_id == upstream_account_id)
            }
        }
    }

    fn descriptor_params(&self) -> BTreeMap<String, String> {
        match self {
            Self::PromptCacheKey(prompt_cache_key) => {
                BTreeMap::from([("promptCacheKey".to_string(), prompt_cache_key.clone())])
            }
            Self::StickyKey {
                sticky_key,
                upstream_account_id,
            } => BTreeMap::from([
                ("stickyKey".to_string(), sticky_key.clone()),
                (
                    "upstreamAccountId".to_string(),
                    upstream_account_id.to_string(),
                ),
            ]),
        }
    }

    fn list_query(&self, page: i64, page_size: i64, snapshot_id: Option<i64>) -> ListQuery {
        let mut query = ListQuery {
            page: Some(page),
            page_size: Some(page_size),
            snapshot_id,
            sort_by: Some("occurredAt".to_string()),
            sort_order: Some("desc".to_string()),
            ..Default::default()
        };
        match self {
            Self::PromptCacheKey(prompt_cache_key) => {
                query.prompt_cache_key = Some(prompt_cache_key.clone());
            }
            Self::StickyKey {
                sticky_key,
                upstream_account_id,
            } => {
                query.sticky_key = Some(sticky_key.clone());
                query.upstream_account_id = Some(*upstream_account_id);
            }
        }
        query
    }
}

impl SubscriptionHub {
    pub(crate) async fn prompt_cache_projection_health(
        &self,
    ) -> PromptCacheTopicProjectionHealthSnapshot {
        let guard = self.state.lock().await;
        let prompt_topics = guard.topics.iter().filter(|(_, cached)| {
            matches!(
                cached.topic,
                SubscriptionTopic::PromptCacheWindow { .. }
                    | SubscriptionTopic::PromptCacheStickyWindow { .. }
                    | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            )
        });
        let mut snapshot = PromptCacheTopicProjectionHealthSnapshot {
            mode: "memory".to_string(),
            response_source: "memory".to_string(),
            recovery_state: "healthy".to_string(),
            ..PromptCacheTopicProjectionHealthSnapshot::default()
        };
        for (topic_key, cached) in prompt_topics {
            let active = guard
                .active_subscribers
                .get(topic_key)
                .copied()
                .unwrap_or_default()
                > 0;
            snapshot.active_topic_count = snapshot
                .active_topic_count
                .saturating_add(u64::from(active));
            snapshot.dirty_key_count = snapshot
                .dirty_key_count
                .saturating_add(cached.prompt_cache_pending_records.len() as u64);
            snapshot.coalesced_event_count = snapshot
                .coalesced_event_count
                .saturating_add(cached.prompt_cache_coalesced_event_count);
            snapshot.full_hydration_count = snapshot
                .full_hydration_count
                .saturating_add(cached.prompt_cache_full_hydration_count);
            snapshot.bounded_key_hydration_count = snapshot
                .bounded_key_hydration_count
                .saturating_add(cached.prompt_cache_bounded_key_hydration_count);
            if active && cached.prompt_cache_response_source == "database_bounded_key_hydrate" {
                snapshot.bounded_cold_recovery_topic_count =
                    snapshot.bounded_cold_recovery_topic_count.saturating_add(1);
            }
            snapshot.live_path_db_read_count = snapshot
                .live_path_db_read_count
                .saturating_add(cached.prompt_cache_full_hydration_count.saturating_sub(1));
            snapshot.baseline_age_ms = snapshot.baseline_age_ms.max(
                cached
                    .prompt_cache_baseline_at
                    .map(|started| started.elapsed().as_millis() as u64)
                    .unwrap_or_default(),
            );
            if active && cached.dirty {
                snapshot.dirty_last_good_topic_count =
                    snapshot.dirty_last_good_topic_count.saturating_add(1);
                if cached.prompt_cache_pressure_deferred {
                    snapshot.pressure_deferred_topic_count =
                        snapshot.pressure_deferred_topic_count.saturating_add(1);
                } else {
                    snapshot.failed_or_stale_topic_count =
                        snapshot.failed_or_stale_topic_count.saturating_add(1);
                }
                snapshot.response_source = "dirty_last_good".to_string();
            } else if cached.prompt_cache_response_source != "memory" {
                snapshot.response_source = cached.prompt_cache_response_source.to_string();
            }
        }
        snapshot.recovery_state = if snapshot.failed_or_stale_topic_count > 0 {
            "failed_or_stale"
        } else if snapshot.live_path_db_read_count > 0 {
            "hot_db_read"
        } else if snapshot.bounded_cold_recovery_topic_count > 0 {
            "bounded_cold_recovery"
        } else if snapshot.pressure_deferred_topic_count > 0 {
            "pressure_deferred"
        } else {
            "healthy"
        }
        .to_string();
        snapshot
    }

    pub(crate) fn new() -> Self {
        let (broadcaster, _) = broadcast::channel(1_024);
        Self {
            state: Mutex::new(SubscriptionHubState::default()),
            broadcaster,
            runtime_mutation_bus: Arc::new(RuntimeMutationBus::new()),
            runtime_topic_recovery_notify: Arc::new(Notify::new()),
            serialization_count: AtomicU64::new(0),
            dashboard_topology_counters: DashboardDeliveryTopologyCounters::default(),
            #[cfg(test)]
            dashboard_topology_sse_frame_observations: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SubscriptionDispatchEvent> {
        self.broadcaster.subscribe()
    }

    pub(crate) fn publish_runtime_mutation(&self, mutation: RuntimeMutation) {
        self.runtime_mutation_bus.publish(mutation);
    }

    pub(crate) fn runtime_mutation_bus_health(&self) -> RuntimeMutationBusHealth {
        self.runtime_mutation_bus.health()
    }

    pub(crate) fn runtime_mutation_bus(&self) -> Arc<RuntimeMutationBus> {
        self.runtime_mutation_bus.clone()
    }

    fn serialize_frame(
        &self,
        descriptor: SubscriptionTopicDescriptor,
        topic_key: String,
        schema_epoch: String,
        cursor: u64,
        payload_bytes: Vec<u8>,
    ) -> Result<SerializedTopicFrame, ApiError> {
        let frame =
            serialize_topic_frame(descriptor, topic_key, schema_epoch, cursor, payload_bytes)?;
        self.serialization_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .record_serialization(&frame.descriptor.topic, frame.retained_bytes());
        self.dashboard_topology_counters
            .record_cursor_advanced(&frame.descriptor.topic);
        Ok(frame)
    }

    #[cfg(test)]
    fn serialization_count(&self) -> u64 {
        self.serialization_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    async fn dashboard_topic_uses_typed_materializer(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        self.state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .is_some_and(|cached| cached.dashboard_materializer.is_some())
    }

    pub(crate) fn dashboard_topology_counters(&self) -> DashboardDeliveryTopologyCounterSnapshot {
        self.dashboard_topology_counters.snapshot()
    }

    pub(crate) async fn dashboard_hot_topic_health(
        &self,
        projection: DashboardRuntimeTopologyCounterSnapshot,
    ) -> DashboardHotTopicsHealthSnapshot {
        let recovery = {
            let guard = self.state.lock().await;
            let mut recovery = DashboardHotTopicRecoveryHealth::default();
            for (topic_key, cached) in &guard.topics {
                if cached.topic.class() != SubscriptionTopicClass::HotProjection
                    || guard
                        .active_subscribers
                        .get(topic_key)
                        .copied()
                        .unwrap_or_default()
                        == 0
                {
                    continue;
                }
                let recovery_pending = guard.runtime_topic_recovery_queued.contains(topic_key)
                    || cached.runtime_topic_recovery_retry_at.is_some();
                let deferred = matches!(
                    cached.topic,
                    SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                ) && (cached.prompt_cache_pressure_deferred
                    || cached.prompt_cache_reconcile_required
                    || !cached.prompt_cache_pending_key_hydrations.is_empty());
                recovery.record(
                    &cached.topic,
                    DashboardHotTopicRecoveryState {
                        degraded: cached.dirty || recovery_pending,
                        deferred,
                    },
                );
            }
            recovery
        };
        self.dashboard_topology_counters
            .hot_topic_health(projection, recovery)
    }

    pub(crate) fn dashboard_delivery_has_degraded_signal(&self) -> bool {
        self.dashboard_topology_counters.has_degraded_signal()
    }

    fn record_dashboard_topology_frame_delivery(&self, frame: &SerializedTopicFrame) {
        self.dashboard_topology_counters
            .record_shared_frame_delivery(&frame.descriptor.topic);
    }

    #[cfg(test)]
    pub(crate) fn reset_dashboard_topology_counters(&self) {
        self.dashboard_topology_counters.reset();
    }

    #[cfg(test)]
    async fn record_dashboard_topology_sse_frame_delivery(
        &self,
        attempt: u64,
        frame: &Arc<SerializedTopicFrame>,
    ) {
        let mut observations = self.dashboard_topology_sse_frame_observations.lock().await;
        observations
            .entry(attempt)
            .or_default()
            .entry(frame.descriptor.topic.clone())
            .or_default()
            .push(frame.clone());
    }

    #[cfg(test)]
    async fn dashboard_topology_sse_frame_observations(
        &self,
        attempt: u64,
    ) -> DashboardTopologyFramesByTopic {
        self.dashboard_topology_sse_frame_observations
            .lock()
            .await
            .get(&attempt)
            .cloned()
            .unwrap_or_default()
    }

    #[cfg(test)]
    async fn reset_dashboard_topology_sse_frame_observations(&self) {
        self.dashboard_topology_sse_frame_observations
            .lock()
            .await
            .clear();
    }

    fn record_dashboard_topology_lag(&self, topic_names: &[String], skipped: u64) {
        for topic_name in topic_names {
            self.dashboard_topology_counters
                .record_lag(topic_name, skipped);
        }
    }

    pub(crate) async fn has_active_topic_name(&self, topic_name: &str) -> bool {
        self.active_topic_subscriber_count(topic_name).await > 0
    }

    pub(crate) async fn active_topic_subscriber_count(&self, topic_name: &str) -> usize {
        let guard = self.state.lock().await;
        guard
            .active_topic_names
            .get(topic_name)
            .copied()
            .unwrap_or_default()
    }

    async fn has_active_topic_key(&self, topic_key: &str) -> bool {
        let guard = self.state.lock().await;
        guard
            .active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default()
            > 0
    }

    pub(crate) fn has_active_topic_name_sync(&self, topic_name: &str) -> bool {
        let Ok(guard) = self.state.try_lock() else {
            return true;
        };
        guard
            .active_topic_names
            .get(topic_name)
            .copied()
            .unwrap_or_default()
            > 0
    }

    pub(crate) async fn mark_topic_name_dirty(&self, topic_name: &str) {
        let mut guard = self.state.lock().await;
        for cached in guard
            .topics
            .values_mut()
            .filter(|cached| cached.topic.name() == topic_name)
        {
            cached.dirty = true;
            cached.latest_live_snapshot = None;
        }
    }

    pub(crate) async fn has_active_dashboard_activity_live_topic(&self) -> bool {
        let guard = self.state.lock().await;
        guard.dashboard_live_subscriber_count > 0
    }

    pub(crate) async fn dashboard_activity_live_subscriber_count(&self) -> usize {
        self.state.lock().await.dashboard_live_subscriber_count
    }

    pub(crate) fn has_active_dashboard_activity_live_topic_sync(&self) -> bool {
        let Ok(guard) = self.state.try_lock() else {
            return true;
        };
        guard.dashboard_live_subscriber_count > 0
    }

    fn register_active_topic_dependencies(
        guard: &mut SubscriptionHubState,
        topic_key: &str,
        topic: &SubscriptionTopic,
    ) {
        for dependency in topic.runtime_topic_dependencies() {
            guard
                .active_topic_dependencies
                .entry(dependency)
                .or_default()
                .insert(topic_key.to_string());
        }
    }

    fn release_active_topic_dependencies(
        guard: &mut SubscriptionHubState,
        topic_key: &str,
        topic: &SubscriptionTopic,
    ) {
        for dependency in topic.runtime_topic_dependencies() {
            let Some(topic_keys) = guard.active_topic_dependencies.get_mut(&dependency) else {
                continue;
            };
            topic_keys.remove(topic_key);
            if topic_keys.is_empty() {
                guard.active_topic_dependencies.remove(&dependency);
            }
        }
    }

    fn active_topic_keys_for_dependency(
        guard: &SubscriptionHubState,
        dependency: &RuntimeTopicDependency,
    ) -> Vec<String> {
        guard
            .active_topic_dependencies
            .get(dependency)
            .map(|topic_keys| topic_keys.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn collect_runtime_topic_work(
        guard: &SubscriptionHubState,
        mutations: &[SequencedRuntimeMutation],
    ) -> Vec<RuntimeTopicWork> {
        let candidate_topic_keys = mutations
            .iter()
            .flat_map(|mutation| mutation.mutation.topic_dependencies())
            .flat_map(|dependency| Self::active_topic_keys_for_dependency(guard, &dependency))
            .collect::<HashSet<_>>();

        candidate_topic_keys
            .into_iter()
            .filter_map(|topic_key| {
                let topic = guard.active_topics.get(&topic_key)?;
                if guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
                    || guard
                        .topics
                        .get(&topic_key)
                        .is_some_and(|cached| cached.dirty)
                    || !mutations
                        .iter()
                        .any(|mutation| topic.is_affected_by_runtime_mutation(&mutation.mutation))
                {
                    return None;
                }
                Some(RuntimeTopicWork {
                    topic: topic.clone(),
                    terminal_event_count: mutations
                        .iter()
                        .filter(|mutation| {
                            topic.is_affected_by_runtime_mutation(&mutation.mutation)
                                && mutation.mutation.is_terminal_invocation()
                        })
                        .count() as u64,
                    includes_invocation_mutation: mutations.iter().any(|mutation| {
                        topic.is_affected_by_runtime_mutation(&mutation.mutation)
                            && mutation.mutation.is_invocation()
                    }),
                })
            })
            .collect()
    }

    async fn register_topic_subscribers(
        self: &Arc<Self>,
        topics: &[SubscriptionTopic],
    ) -> Result<TopicSubscriptionLease, ApiError> {
        let owns_dashboard_live = topics.iter().any(|topic| {
            topic.uses_dashboard_activity_live_overlay()
                || topic.uses_summary_live_overlay()
                || topic.uses_timeseries_live_projection()
                || topic.uses_dashboard_network_live_snapshot()
        });
        let topic_keys = topics
            .iter()
            .map(SubscriptionTopic::cache_key)
            .collect::<Result<HashSet<_>, _>>()?
            .into_iter()
            .collect::<Vec<_>>();
        let topic_names = topics
            .iter()
            .map(SubscriptionTopic::name)
            .collect::<HashSet<_>>()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut guard = self.state.lock().await;
        for topic in topics {
            let topic_key = topic.cache_key()?;
            guard.active_topics.insert(topic_key.clone(), topic.clone());
            Self::register_active_topic_dependencies(&mut guard, &topic_key, topic);
        }
        for topic_key in &topic_keys {
            let active_subscriber_count = {
                let count = guard
                    .active_subscribers
                    .entry(topic_key.clone())
                    .or_insert(0);
                *count += 1;
                *count
            };
            if let Some(topic) = guard.active_topics.get(topic_key) {
                self.dashboard_topology_counters
                    .set_active_subscriber_count(topic.name(), active_subscriber_count);
                if active_subscriber_count > 1 && guard.topics.contains_key(topic_key) {
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                }
            }
        }
        for topic_name in &topic_names {
            *guard
                .active_topic_names
                .entry(topic_name.clone())
                .or_insert(0) += 1;
        }
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_add(usize::from(owns_dashboard_live));
        Ok(TopicSubscriptionLease {
            hub: self.clone(),
            topic_keys,
            topic_names,
            owns_dashboard_live,
        })
    }

    #[cfg(test)]
    pub(crate) async fn register_test_topic_name(
        self: &Arc<Self>,
        topic_name: &str,
    ) -> TopicSubscriptionLease {
        let mut guard = self.state.lock().await;
        *guard
            .active_topic_names
            .entry(topic_name.to_string())
            .or_insert(0) += 1;
        let owns_dashboard_live = topic_name == "dashboard.activity.current"
            || guard.topics.values().any(|cached| {
                cached.topic.name() == topic_name
                    && (cached.topic.uses_dashboard_activity_live_overlay()
                        || cached.topic.uses_summary_live_overlay()
                        || cached.topic.uses_timeseries_live_projection()
                        || cached.topic.uses_dashboard_network_live_snapshot())
            });
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_add(usize::from(owns_dashboard_live));
        let topic_keys = guard
            .topics
            .iter()
            .filter(|(_, cached)| cached.topic.name() == topic_name)
            .map(|(topic_key, _)| topic_key.clone())
            .collect::<Vec<_>>();
        for topic_key in &topic_keys {
            if let Some(topic) = guard
                .topics
                .get(topic_key)
                .map(|cached| cached.topic.clone())
            {
                guard.active_topics.insert(topic_key.clone(), topic.clone());
                Self::register_active_topic_dependencies(&mut guard, topic_key, &topic);
            }
            let active_subscriber_count = {
                let count = guard
                    .active_subscribers
                    .entry(topic_key.clone())
                    .or_insert(0);
                *count += 1;
                *count
            };
            if let Some(topic) = guard.active_topics.get(topic_key) {
                self.dashboard_topology_counters
                    .set_active_subscriber_count(topic.name(), active_subscriber_count);
                if active_subscriber_count > 1 && guard.topics.contains_key(topic_key) {
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                }
            }
        }
        TopicSubscriptionLease {
            hub: self.clone(),
            topic_keys,
            topic_names: vec![topic_name.to_string()],
            owns_dashboard_live,
        }
    }

    async fn release_topic_subscribers(
        &self,
        topic_keys: Vec<String>,
        topic_names: Vec<String>,
        owns_dashboard_live: bool,
    ) {
        let mut guard = self.state.lock().await;
        for topic_key in topic_keys {
            let topic_name = guard
                .active_topics
                .get(&topic_key)
                .or_else(|| guard.topics.get(&topic_key).map(|cached| &cached.topic))
                .map(|topic| topic.name());
            let active_subscriber_count = match guard.active_subscribers.get_mut(&topic_key) {
                Some(count) if *count > 1 => {
                    *count -= 1;
                    *count
                }
                Some(_) => {
                    guard.active_subscribers.remove(&topic_key);
                    if let Some(topic) = guard.active_topics.remove(&topic_key) {
                        Self::release_active_topic_dependencies(&mut guard, &topic_key, &topic);
                    }
                    guard.prompt_cache_prebaseline_records.remove(&topic_key);
                    guard
                        .prompt_cache_prebaseline_key_hydrations
                        .remove(&topic_key);
                    guard.parallel_work_prebaseline_mutations.remove(&topic_key);
                    guard.runtime_topic_recovery_generation =
                        guard.runtime_topic_recovery_generation.saturating_add(1);
                    let recovery_generation = guard.runtime_topic_recovery_generation;
                    if let Some(cached) = guard.topics.get_mut(&topic_key) {
                        // A disconnected owner cannot consume the mutation stream. Rebuild this
                        // selection before it can resume instead of scanning retained caches for
                        // every runtime event.
                        cached.dirty = true;
                        cached.runtime_topic_recovery_generation = recovery_generation;
                        cached.runtime_topic_recovery_retry_at = None;
                        cached.refresh_scheduled = false;
                        cached.latest_live_snapshot = None;
                        if matches!(
                            cached.topic,
                            SubscriptionTopic::PromptCacheWindow { .. }
                                | SubscriptionTopic::PromptCacheStickyWindow { .. }
                                | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                        ) {
                            cached.prompt_cache_pending_records.clear();
                            cached.prompt_cache_applied_terminal_ids.clear();
                            cached.prompt_cache_refresh_scheduled = false;
                            cached.prompt_cache_baseline_at = None;
                        }
                    }
                    0
                }
                None => 0,
            };
            if let Some(topic_name) = topic_name {
                self.dashboard_topology_counters
                    .set_active_subscriber_count(topic_name, active_subscriber_count);
            }
        }
        for topic_name in topic_names {
            match guard.active_topic_names.get_mut(&topic_name) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    guard.active_topic_names.remove(&topic_name);
                }
                None => {}
            }
        }
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_sub(usize::from(owns_dashboard_live));
    }

    async fn register_server_push_topics(
        self: &Arc<Self>,
        state: Arc<AppState>,
        topics: Vec<SubscriptionTopic>,
    ) -> Result<ServerPushTopicLease, ApiError> {
        let mut unique_topics = HashMap::new();
        for topic in topics {
            unique_topics.entry(topic.cache_key()?).or_insert(topic);
        }

        let topic_keys = unique_topics.keys().cloned().collect::<Vec<_>>();
        let mut topics_to_start = Vec::new();
        {
            let mut guard = self.state.lock().await;
            for (topic_key, topic) in unique_topics {
                *guard
                    .server_push_subscribers
                    .entry(topic_key.clone())
                    .or_insert(0) += 1;
                if guard.server_push_tasks.insert(topic_key.clone()) {
                    topics_to_start.push((topic_key, topic));
                }
            }
        }

        for (topic_key, topic) in topics_to_start {
            let hub = self.clone();
            let state = state.clone();
            tokio::spawn(async move {
                run_server_push_topic_loop(hub, state, topic_key, topic).await;
            });
        }

        Ok(ServerPushTopicLease {
            hub: self.clone(),
            topic_keys,
        })
    }

    async fn release_server_push_topics(&self, topic_keys: Vec<String>) {
        let mut guard = self.state.lock().await;
        for topic_key in topic_keys {
            match guard.server_push_subscribers.get_mut(&topic_key) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    guard.server_push_subscribers.remove(&topic_key);
                }
                None => {}
            }
        }
    }

    async fn stop_server_push_task_if_idle(&self, topic_key: &str) -> bool {
        let mut guard = self.state.lock().await;
        if guard
            .server_push_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or(0)
            > 0
        {
            return false;
        }
        guard.server_push_tasks.remove(topic_key);
        true
    }

    async fn clear_server_push_task(&self, topic_key: &str) {
        let mut guard = self.state.lock().await;
        guard.server_push_tasks.remove(topic_key);
    }

    pub(crate) async fn prepare_connection(
        &self,
        state: Arc<AppState>,
        descriptors: Vec<SubscriptionTopicDescriptor>,
        resume: Vec<SubscriptionResumeCursor>,
    ) -> Result<PreparedSubscriptionConnection, ApiError> {
        let resume_by_topic_key = resume
            .into_iter()
            .map(|item| (item.topic_key.clone(), item))
            .collect::<HashMap<_, _>>();
        let mut initial = Vec::new();
        let mut last_sent_cursors = HashMap::new();
        let mut outcomes = Vec::new();

        for descriptor in descriptors {
            let topic = SubscriptionTopic::from_descriptor(&descriptor)?;
            let cached = self
                .ensure_cached_topic(state.clone(), topic.clone())
                .await?;
            let topic_key = topic.cache_key()?;
            let resume_cursor = resume_by_topic_key.get(&topic_key);
            let continuity_reset = resume_cursor
                .zip(cached.continuity_reset_cursor)
                .is_some_and(|(resume, reset_cursor)| resume.cursor < reset_cursor);
            let replay_attempt = if continuity_reset {
                Err(ReplayMissReason::ContinuityReset)
            } else {
                self.replay_events_for_resume(&topic_key, topic.schema_epoch(), resume_cursor)
                    .await
            };

            match replay_attempt {
                Ok(Some(events)) if !events.is_empty() => {
                    let replay_event_count = events.len();
                    let replay_bytes = events.iter().map(|event| event.bytes).sum::<usize>();
                    tracing::debug!(
                        topic_key,
                        replay_event_count,
                        replay_bytes,
                        "subscription replay hit"
                    );
                    for event in events {
                        initial.push(PreparedTopicFrame {
                            frame: event.frame,
                            kind: TopicFrameKind::Replay,
                        });
                    }
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::ReplayHit,
                        replay_event_count,
                        replay_bytes,
                        cursor: cached.cursor,
                        miss_reason: None,
                    });
                }
                Ok(Some(_)) => {
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::ResumeCaughtUp,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: None,
                    });
                }
                Ok(None) => {
                    initial.push(PreparedTopicFrame {
                        frame: cached.snapshot_frame.clone(),
                        kind: TopicFrameKind::Snapshot,
                    });
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::SnapshotNoResume,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: None,
                    });
                }
                Err(reason) => {
                    if resume_cursor.is_some() {
                        self.dashboard_topology_counters
                            .record_reconnect_churn(topic.name());
                    }
                    tracing::debug!(
                        topic_key,
                        miss_reason = reason.as_str(),
                        "subscription replay miss, falling back to snapshot"
                    );
                    initial.push(PreparedTopicFrame {
                        frame: cached.snapshot_frame.clone(),
                        kind: TopicFrameKind::Snapshot,
                    });
                    last_sent_cursors.insert(topic_key.clone(), cached.cursor);
                    outcomes.push(TopicInitOutcome {
                        topic_key: topic_key.clone(),
                        disposition: TopicInitDisposition::SnapshotResumeMiss,
                        replay_event_count: 0,
                        replay_bytes: 0,
                        cursor: cached.cursor,
                        miss_reason: Some(reason.as_str()),
                    });
                }
            }
            if cached.dirty {
                self.schedule_dirty_topic_recovery(state.clone(), topic.clone())
                    .await;
            }
        }

        Ok(PreparedSubscriptionConnection {
            initial,
            last_sent_cursors,
            outcomes,
        })
    }

    async fn replay_events_for_resume(
        &self,
        topic_key: &str,
        schema_epoch: String,
        resume: Option<&SubscriptionResumeCursor>,
    ) -> Result<Option<Vec<ReplayableTopicEvent>>, ReplayMissReason> {
        let Some(resume) = resume else {
            return Ok(None);
        };

        if resume.schema_epoch != schema_epoch {
            return Err(ReplayMissReason::SchemaEpochMismatch);
        }

        let guard = self.state.lock().await;
        let Some(cached) = guard.topics.get(topic_key) else {
            return Err(ReplayMissReason::UnknownTopic);
        };

        let mut gap = Vec::new();
        let mut gap_bytes = 0usize;
        let mut matched = false;

        for event in &cached.replay_events {
            if event.frame.cursor <= resume.cursor {
                matched = true;
                continue;
            }
            if !matched
                && resume.cursor > 0
                && event.frame.cursor > resume.cursor
                && cached
                    .replay_events
                    .front()
                    .is_some_and(|front| front.frame.cursor > resume.cursor)
            {
                return Err(ReplayMissReason::GapWindowMiss);
            }
            gap_bytes = gap_bytes.saturating_add(event.bytes);
            if gap.len() + 1 > SUBSCRIPTION_REPLAY_MAX_GAP_EVENTS {
                return Err(ReplayMissReason::GapEventBudgetExceeded);
            }
            if gap_bytes > SUBSCRIPTION_REPLAY_MAX_GAP_BYTES {
                return Err(ReplayMissReason::GapByteBudgetExceeded);
            }
            gap.push(event.clone());
        }

        if resume.cursor > 0
            && cached
                .replay_events
                .front()
                .is_some_and(|front| front.frame.cursor > resume.cursor)
        {
            return Err(ReplayMissReason::GapWindowMiss);
        }

        Ok(Some(gap))
    }

    async fn ensure_cached_topic(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        let topic_key = topic.cache_key()?;
        let (existing, has_active_owner) = {
            let guard = self.state.lock().await;
            (
                guard.topics.get(&topic_key).cloned(),
                guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0,
            )
        };
        if let Some(existing) = existing
            && ((has_active_owner
                && existing.dirty
                && !existing
                    .dashboard_materializer
                    .as_ref()
                    .is_some_and(DashboardTopicMaterializer::requires_terminal_window_rebase))
                || (!existing.dirty
                    && (!topic.is_closed_summary_topic()
                        || existing.calendar_anchor == subscription_calendar_anchor(&topic))))
        {
            return Ok(existing);
        }
        if !has_active_owner {
            #[cfg(test)]
            return self.refresh_topic(state, topic, false).await;
            #[cfg(not(test))]
            return Err(ApiError::from(anyhow!(
                "subscription topic setup requires an active owner"
            )));
        }
        for _ in 0..SUBSCRIPTION_INITIAL_TOPIC_BUILD_ATTEMPTS {
            if let Some(cached) = self
                .refresh_topic_if_active(state.clone(), topic.clone(), false)
                .await?
            {
                return Ok(cached);
            }
            if let Some(existing) = self.state.lock().await.topics.get(&topic_key).cloned() {
                return Ok(existing);
            }
        }
        Err(ApiError::from(anyhow!(
            "subscription topic changed before its bounded initial snapshot was ready"
        )))
    }

    async fn refresh_topic(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
    ) -> Result<CachedSubscriptionTopic, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, false)
            .await
            .map(|cached| cached.expect("unguarded topic refresh should always commit"))
    }

    async fn refresh_topic_if_active(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        self.refresh_topic_inner(state, topic, emit_live, true)
            .await
    }

    async fn build_prompt_cache_consistent_baseline(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(BuiltSubscriptionTopicPayload, PromptCacheBaselineBuild), ApiError> {
        if let SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size,
            recent_invocation_limit,
            blocked_binding_upstream_account_id,
            blocked_binding_constraint_source,
        } = topic
        {
            let mut transaction = state.pool.begin().await?;
            let baseline_row_id =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(transaction.as_mut())
                    .await?;
            let snapshot_at = Utc::now();
            let blocked_binding_filter = PromptCacheConversationBlockedBindingFilter {
                upstream_account_id: *blocked_binding_upstream_account_id,
                constraint_source: *blocked_binding_constraint_source,
            };
            let blocked_binding_filter = blocked_binding_filter
                .is_active()
                .then_some(blocked_binding_filter);
            let (response, runtime_overlay_terminal_identities) = build_prompt_cache_conversations_response_for_request_with_runtime_overlay_terminal_identities_on_connection(
                state.as_ref(),
                PromptCacheConversationsRequest {
                    selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                        SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                    ),
                    detail_level: PromptCacheConversationDetailLevel::Full,
                    recent_invocation_limit: Some(*recent_invocation_limit),
                    page_size: Some(*page_size),
                    cursor: None,
                    snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
                    blocked_binding_filter: blocked_binding_filter.clone(),
                },
                transaction.as_mut(),
                snapshot_at,
                Some(baseline_row_id),
            )
            .await?;
            let payload = BuiltSubscriptionTopicPayload::Dashboard(
                DashboardTopicMaterializer::WorkingConversations {
                    state: Arc::new(StdMutex::new(
                        DashboardWorkingConversationsMaterializerState::new(
                            response,
                            *page_size,
                            *recent_invocation_limit,
                            blocked_binding_filter,
                        ),
                    )),
                },
            );
            let candidate_identities = {
                let guard = self.state.lock().await;
                let topic_key = topic.cache_key()?;
                guard
                    .prompt_cache_prebaseline_records
                    .get(&topic_key)
                    .into_iter()
                    .flat_map(|records| records.values())
                    .chain(
                        guard
                            .topics
                            .get(&topic_key)
                            .into_iter()
                            .flat_map(|cached| cached.prompt_cache_pending_records.values()),
                    )
                    .map(|delta| delta.identity.clone())
                    .collect::<HashSet<_>>()
            };
            let persisted_identities = load_persisted_invocation_identities(
                transaction.as_mut(),
                &candidate_identities,
                baseline_row_id,
            )
            .await?;
            transaction.commit().await?;
            return Ok((
                payload,
                PromptCacheBaselineBuild {
                    baseline_row_id,
                    persisted_identities,
                    runtime_overlay_terminal_identities,
                },
            ));
        }

        for _ in 0..3 {
            let mut observer = state.pool.acquire().await?;
            let version_before = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            let baseline_row_id =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(&mut *observer)
                    .await?;
            let payload = topic.build_cached_payload(state.clone()).await?;
            let candidate_identities = {
                let guard = self.state.lock().await;
                let topic_key = topic.cache_key()?;
                guard
                    .prompt_cache_prebaseline_records
                    .get(&topic_key)
                    .into_iter()
                    .flat_map(|records| records.values())
                    .chain(
                        guard
                            .topics
                            .get(&topic_key)
                            .into_iter()
                            .flat_map(|cached| cached.prompt_cache_pending_records.values()),
                    )
                    .map(|delta| delta.identity.clone())
                    .collect::<HashSet<_>>()
            };
            let persisted_identities = load_persisted_invocation_identities(
                &mut observer,
                &candidate_identities,
                baseline_row_id,
            )
            .await?;
            let version_after = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            if version_before == version_after {
                return Ok((
                    payload,
                    PromptCacheBaselineBuild {
                        baseline_row_id,
                        persisted_identities,
                        runtime_overlay_terminal_identities: HashSet::new(),
                    },
                ));
            }
        }
        Err(ApiError::from(anyhow!(
            "prompt cache baseline changed during build"
        )))
    }

    async fn build_parallel_work_consistent_baseline(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(BuiltSubscriptionTopicPayload, ParallelWorkBaselineBuild), ApiError> {
        for _ in 0..3 {
            let mut observer = state.pool.acquire().await?;
            let version_before = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            let baseline_row_id =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(&mut *observer)
                    .await?;
            let payload = topic.build_cached_payload(state.clone()).await?;
            let candidate_identities = {
                let guard = self.state.lock().await;
                let topic_key = topic.cache_key()?;
                let mut identities = guard
                    .parallel_work_prebaseline_mutations
                    .get(&topic_key)
                    .into_iter()
                    .flat_map(|mutations| mutations.values())
                    .map(|mutation| {
                        format!(
                            "{}\0{}",
                            mutation.identity.invoke_id, mutation.identity.occurred_at
                        )
                    })
                    .collect::<HashSet<_>>();
                if let Some(DashboardTopicMaterializer::ParallelWork { base }) = guard
                    .topics
                    .get(&topic_key)
                    .and_then(|cached| cached.dashboard_materializer.as_ref())
                {
                    identities.extend(
                        base.lock()
                            .expect("parallel-work materializer state lock")
                            .runtime_mutations
                            .keys()
                            .cloned(),
                    );
                }
                identities
            };
            let persisted_identities = load_persisted_invocation_identities(
                &mut observer,
                &candidate_identities,
                baseline_row_id,
            )
            .await?;
            let version_after = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
                .fetch_one(&mut *observer)
                .await?;
            if version_before == version_after {
                return Ok((
                    payload,
                    ParallelWorkBaselineBuild {
                        persisted_identities,
                    },
                ));
            }
        }
        Err(ApiError::from(anyhow!(
            "parallel-work baseline changed during build"
        )))
    }

    async fn refresh_topic_inner(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        emit_live: bool,
        require_active_owner: bool,
    ) -> Result<Option<CachedSubscriptionTopic>, ApiError> {
        let topic_key = topic.cache_key()?;
        let schema_epoch = topic.schema_epoch();
        let descriptor = topic.descriptor();
        let started = Instant::now();
        let is_prompt_cache_topic = matches!(
            topic,
            SubscriptionTopic::PromptCacheWindow { .. }
                | SubscriptionTopic::PromptCacheStickyWindow { .. }
                | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
        );
        let is_open_parallel_work_topic = matches!(
            &topic,
            SubscriptionTopic::ParallelWorkCurrent { range, .. } if range != "yesterday"
        );
        // A recovery or owner disconnect may happen while a cold build is in flight. Capture
        // the cache generation before building so an old result can never clear newer dirty
        // state or replace the retained last-good frame.
        let (refresh_generation, refresh_had_cached_topic) = if require_active_owner {
            let guard = self.state.lock().await;
            if guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                return Ok(None);
            }
            guard
                .topics
                .get(&topic_key)
                .map(|cached| (Some(cached.runtime_topic_recovery_generation), true))
                .unwrap_or((Some(guard.runtime_topic_recovery_generation), false))
        } else {
            (None, false)
        };
        let (mut built_payload, prompt_cache_build, parallel_work_build) = if is_prompt_cache_topic
        {
            let (payload, build) = self
                .build_prompt_cache_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, Some(build), None)
        } else if is_open_parallel_work_topic {
            let (payload, build) = self
                .build_parallel_work_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, None, Some(build))
        } else {
            (topic.build_cached_payload(state.clone()).await?, None, None)
        };
        self.dashboard_topology_counters.record_materialization(
            topic.name(),
            matches!(&built_payload, BuiltSubscriptionTopicPayload::Json(_))
                && topic.is_unmigrated_dashboard_hot_projection(),
        );

        let (cached, dispatch, initial_hydration, initial_reconcile) = {
            let mut guard = self.state.lock().await;
            if require_active_owner {
                let active = guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0;
                let generation_matches = if refresh_had_cached_topic {
                    guard.topics.get(&topic_key).is_some_and(|cached| {
                        Some(cached.runtime_topic_recovery_generation) == refresh_generation
                    })
                } else {
                    Some(guard.runtime_topic_recovery_generation) == refresh_generation
                        && guard.topics.get(&topic_key).is_none_or(|cached| {
                            Some(cached.runtime_topic_recovery_generation) == refresh_generation
                        })
                };
                if !active || !generation_matches {
                    // A newer gap or owner generation owns the cache now. Only the owner that
                    // observed this generation may dirty it; never invalidate a newer clean
                    // frame committed by a reconnecting subscriber.
                    if !active && let Some(cached) = guard.topics.get_mut(&topic_key) {
                        cached.dirty = true;
                        cached.refresh_scheduled = false;
                        cached.latest_live_snapshot = None;
                    }
                    return Ok(None);
                }
            }
            if let BuiltSubscriptionTopicPayload::Json(payload) = &mut built_payload
                && let Some(live) = guard
                    .topics
                    .get(&topic_key)
                    .and_then(|cached| cached.latest_live_snapshot.as_ref())
                    .cloned()
            {
                apply_topic_live_overlay_to_payload(state.as_ref(), &topic, payload, &live)?;
            }
            if let BuiltSubscriptionTopicPayload::Dashboard(
                DashboardTopicMaterializer::ParallelWork { base },
            ) = &built_payload
            {
                let baseline_identities = parallel_work_build
                    .as_ref()
                    .map(|build| build.persisted_identities.clone())
                    .unwrap_or_default();
                let mut replay = guard
                    .topics
                    .get(&topic_key)
                    .and_then(|cached| match cached.dashboard_materializer.as_ref() {
                        Some(DashboardTopicMaterializer::ParallelWork { base }) => Some(
                            base.lock()
                                .expect("parallel-work materializer state lock")
                                .runtime_mutations
                                .clone(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_default();
                let mut pending = guard
                    .parallel_work_prebaseline_mutations
                    .remove(&topic_key)
                    .unwrap_or_default();
                replay.append(&mut pending);
                let mut base = base.lock().expect("parallel-work materializer state lock");
                base.baseline_identities = baseline_identities;
                let mut unresolved = BTreeMap::new();
                for (identity, mutation) in &replay {
                    if base
                        .apply_runtime_mutation(mutation)
                        .needs_account_reconcile
                    {
                        unresolved.insert(identity.clone(), mutation.clone());
                    }
                }
                if !unresolved.is_empty() {
                    guard
                        .parallel_work_prebaseline_mutations
                        .entry(topic_key.clone())
                        .or_default()
                        .extend(unresolved);
                }
            }
            let mut prompt_cache_pending = guard
                .prompt_cache_prebaseline_records
                .remove(&topic_key)
                .unwrap_or_default();
            let prebaseline_working_hydration_keys = guard
                .prompt_cache_prebaseline_key_hydrations
                .remove(&topic_key)
                .unwrap_or_default();
            if let Some(existing) = guard.topics.get_mut(&topic_key) {
                prompt_cache_pending.append(&mut existing.prompt_cache_pending_records);
                existing.prompt_cache_refresh_scheduled = false;
            }
            let prompt_cache_replay = prompt_cache_build
                .as_ref()
                .map(|build| {
                    prompt_cache_pending
                        .into_values()
                        .filter(|delta| {
                            prompt_cache_delta_needs_replay(delta, &build.persisted_identities)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut prompt_cache_applied_terminal_ids = prompt_cache_build
                .as_ref()
                .map(|build| build.runtime_overlay_terminal_identities.clone())
                .unwrap_or_default();
            let mut deferred_working_replay = Vec::new();
            let mut deferred_working_hydration_keys = prebaseline_working_hydration_keys;
            let mut deferred_working_reconcile = false;
            if !prompt_cache_replay.is_empty() {
                let baseline_row_id = prompt_cache_build
                    .as_ref()
                    .map_or(0, |build| build.baseline_row_id);
                match &mut built_payload {
                    BuiltSubscriptionTopicPayload::Json(payload) => {
                        apply_prompt_cache_records_to_payload(
                            &topic,
                            payload,
                            &prompt_cache_replay,
                            &mut prompt_cache_applied_terminal_ids,
                            baseline_row_id,
                        )?;
                    }
                    BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::WorkingConversations { state },
                    ) => {
                        let update = state
                            .lock()
                            .expect("working conversations materializer state lock")
                            .apply_deltas(
                                &prompt_cache_replay,
                                &mut prompt_cache_applied_terminal_ids,
                                baseline_row_id,
                            )?;
                        match update {
                            WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(
                                keys,
                            ) => {
                                // The cursor-consistent base is valid, but its compact replay
                                // cannot represent this key exactly. Commit the base and retain
                                // every replay record so the bounded hydrator can replace just
                                // the affected key without losing an initial live mutation.
                                deferred_working_replay = prompt_cache_replay;
                                deferred_working_hydration_keys = keys;
                            }
                            WorkingConversationsProjectionUpdate::NeedsReconcile => {
                                // An incomplete delta cannot be proven safe to apply. Preserve
                                // it on the committed cache and use the existing bounded
                                // recovery path rather than failing setup after removing it.
                                deferred_working_replay = prompt_cache_replay;
                                deferred_working_reconcile = true;
                            }
                            WorkingConversationsProjectionUpdate::Changed
                            | WorkingConversationsProjectionUpdate::Unchanged => {}
                        }
                    }
                    BuiltSubscriptionTopicPayload::Dashboard(_) => {}
                }
            }
            let serialized_payload = built_payload.serialize(
                guard.dashboard_current_slice.as_deref(),
                guard.dashboard_network_slice.as_deref(),
                guard.dashboard_terminal_slice.as_deref(),
            )?;
            let refreshed_dashboard_materializer = built_payload.dashboard_materializer();
            let current_slice = guard.dashboard_current_slice.clone();
            let network_slice = guard.dashboard_network_slice.clone();
            let terminal_slice = guard.dashboard_terminal_slice.clone();
            if deferred_working_replay.is_empty()
                && let Some(existing) = guard.topics.get_mut(&topic_key)
            {
                if existing.snapshot_frame.payload_bytes.as_ref() == serialized_payload.as_slice()
                    && existing.dirty
                    && existing.dashboard_materializer.is_some()
                    && refreshed_dashboard_materializer.is_some()
                {
                    reuse_unchanged_cached_topic(existing, &serialized_payload)
                        .expect("matching dashboard payload must reuse the cached topic");
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                    existing.dashboard_materializer = refreshed_dashboard_materializer.clone();
                    existing.dashboard_base_revision = existing.cursor;
                    existing.dashboard_materialized_revision = refreshed_dashboard_materializer
                        .as_ref()
                        .and_then(|materializer| {
                            materializer.revision(
                                existing.cursor,
                                current_slice.as_deref(),
                                network_slice.as_deref(),
                                terminal_slice.as_deref(),
                            )
                        });
                    existing.snapshot_payload = built_payload.snapshot_payload();
                    if let Some(build) = &prompt_cache_build {
                        finish_prompt_cache_baseline_reuse(
                            existing,
                            build,
                            &prompt_cache_applied_terminal_ids,
                        );
                    }
                    return Ok(Some(existing.clone()));
                }
                if reuse_unchanged_cached_topic(existing, &serialized_payload).is_some() {
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                    if let Some(build) = &prompt_cache_build {
                        finish_prompt_cache_baseline_reuse(
                            existing,
                            build,
                            &prompt_cache_applied_terminal_ids,
                        );
                    }
                    return Ok(Some(existing.clone()));
                }
            }
            let current_cursor = guard.topics.get(&topic_key).map_or(0, |entry| entry.cursor);
            let next_cursor = current_cursor.saturating_add(1);
            let continuity_reset_cursor = guard.topics.get(&topic_key).and_then(|entry| {
                if entry.dirty {
                    Some(next_cursor)
                } else {
                    entry.continuity_reset_cursor
                }
            });
            let frame = Arc::new(self.serialize_frame(
                descriptor.clone(),
                topic_key.clone(),
                schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let payload_bytes = frame.payload_bytes.len();
            let dashboard_materializer = refreshed_dashboard_materializer;
            let dashboard_materialized_revision =
                dashboard_materializer.as_ref().and_then(|materializer| {
                    materializer.revision(
                        next_cursor,
                        guard.dashboard_current_slice.as_deref(),
                        guard.dashboard_network_slice.as_deref(),
                        guard.dashboard_terminal_slice.as_deref(),
                    )
                });
            let had_key_hydration_scheduled = guard
                .topics
                .get(&topic_key)
                .is_some_and(|entry| entry.prompt_cache_key_hydration_scheduled);
            let had_reconcile_scheduled = guard
                .topics
                .get(&topic_key)
                .is_some_and(|entry| entry.prompt_cache_reconcile_scheduled);
            let schedule_initial_hydration =
                !deferred_working_hydration_keys.is_empty() && !had_key_hydration_scheduled;
            let schedule_initial_reconcile = deferred_working_reconcile && !had_reconcile_scheduled;
            let mut next = CachedSubscriptionTopic {
                topic: topic.clone(),
                descriptor: descriptor.clone(),
                schema_epoch: schema_epoch.clone(),
                cursor: next_cursor,
                snapshot_built_at: Instant::now(),
                refresh_scheduled: false,
                conversation_overview_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.conversation_overview_refresh_scheduled),
                conversation_overview_refresh_in_flight: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.conversation_overview_refresh_in_flight),
                conversation_overview_refresh_pending: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.conversation_overview_refresh_pending),
                dirty: deferred_working_reconcile,
                runtime_topic_recovery_generation: guard
                    .topics
                    .get(&topic_key)
                    .map_or(guard.runtime_topic_recovery_generation, |entry| {
                        entry.runtime_topic_recovery_generation
                    }),
                runtime_topic_recovery_retry_at: None,
                summary_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.summary_refresh_scheduled),
                summary_refresh_in_flight: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.summary_refresh_in_flight),
                summary_pending_event_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.summary_pending_event_count),
                summary_retry_backoff_ms: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.summary_retry_backoff_ms),
                parallel_work_refresh_scheduled: false,
                prompt_cache_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.prompt_cache_refresh_scheduled),
                prompt_cache_reconcile_scheduled: had_reconcile_scheduled
                    || deferred_working_reconcile,
                prompt_cache_key_hydration_scheduled: had_key_hydration_scheduled
                    || !deferred_working_hydration_keys.is_empty(),
                prompt_cache_pending_records: deferred_working_replay
                    .into_iter()
                    .map(|record| (record.identity.clone(), record))
                    .collect(),
                prompt_cache_pending_key_hydrations: deferred_working_hydration_keys,
                prompt_cache_candidate_refill_required: false,
                prompt_cache_applied_terminal_ids: if matches!(
                    topic,
                    SubscriptionTopic::PromptCacheWindow { .. }
                        | SubscriptionTopic::PromptCacheStickyWindow { .. }
                        | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                ) {
                    prompt_cache_applied_terminal_ids
                } else {
                    guard
                        .topics
                        .get(&topic_key)
                        .map(|entry| entry.prompt_cache_applied_terminal_ids.clone())
                        .unwrap_or_default()
                },
                prompt_cache_coalesced_event_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.prompt_cache_coalesced_event_count),
                prompt_cache_full_hydration_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(1, |entry| {
                        entry.prompt_cache_full_hydration_count.saturating_add(1)
                    }),
                prompt_cache_bounded_key_hydration_count: guard
                    .topics
                    .get(&topic_key)
                    .map_or(0, |entry| entry.prompt_cache_bounded_key_hydration_count),
                prompt_cache_baseline_at: Some(Instant::now()),
                prompt_cache_baseline_row_id: prompt_cache_build
                    .as_ref()
                    .map_or(0, |build| build.baseline_row_id),
                prompt_cache_response_source: if guard.topics.contains_key(&topic_key) {
                    "database_reconcile"
                } else {
                    "initial_baseline"
                },
                prompt_cache_reconcile_required: deferred_working_reconcile,
                prompt_cache_pressure_deferred: false,
                latest_live_snapshot: guard
                    .topics
                    .get(&topic_key)
                    .and_then(|entry| entry.latest_live_snapshot.clone()),
                calendar_anchor: subscription_calendar_anchor(&topic),
                continuity_reset_cursor,
                dashboard_materializer,
                dashboard_base_revision: next_cursor,
                dashboard_materialized_revision,
                snapshot_payload: built_payload.snapshot_payload(),
                snapshot_frame: frame.clone(),
                snapshot_bytes: payload_bytes,
                replay_events: guard
                    .topics
                    .get(&topic_key)
                    .filter(|entry| !entry.dirty)
                    .map(|entry| entry.replay_events.clone())
                    .unwrap_or_default(),
                replay_bytes: guard
                    .topics
                    .get(&topic_key)
                    .filter(|entry| !entry.dirty)
                    .map_or(0, |entry| entry.replay_bytes),
            };
            if emit_live {
                let retained_bytes = frame.retained_bytes();
                let replay_event = ReplayableTopicEvent {
                    frame: frame.clone(),
                    bytes: retained_bytes,
                    emitted_at: Utc::now(),
                };
                next.replay_events.push_back(replay_event);
                next.replay_bytes = next.replay_bytes.saturating_add(retained_bytes);
                prune_replay_window(&mut next.replay_events, &mut next.replay_bytes);
            }
            guard.topics.insert(topic_key.clone(), next.clone());
            let dispatch = emit_live.then_some(SubscriptionDispatchEvent { frame });
            (
                next,
                dispatch,
                schedule_initial_hydration.then(|| topic.clone()),
                schedule_initial_reconcile.then(|| topic.clone()),
            )
        };

        tracing::debug!(
            topic_key,
            schema_epoch,
            emit_live,
            snapshot_build_ms = started.elapsed().as_millis() as u64,
            payload_bytes = cached.snapshot_bytes,
            "subscription topic snapshot built"
        );

        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch.clone());
            tracing::debug!(
                topic_key = dispatch.frame.topic_key,
                cursor = dispatch.frame.cursor,
                fanout_receivers = self.broadcaster.receiver_count(),
                "subscription topic live event dispatched"
            );
        }
        if let Some(topic) = initial_hydration {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        if let Some(topic) = initial_reconcile {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }

        Ok(Some(cached))
    }

    pub(crate) async fn materialize_dashboard_current_slice(
        &self,
        slice: DashboardCurrentProjectionSlice,
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            if guard
                .dashboard_current_slice
                .as_ref()
                .is_some_and(|current| current.revision >= slice.revision)
            {
                return;
            }
            guard.dashboard_current_slice = Some(Arc::new(slice));
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    pub(crate) async fn materialize_dashboard_network_slice(
        &self,
        slice: DashboardNetworkProjectionSlice,
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            if guard
                .dashboard_network_slice
                .as_ref()
                .is_some_and(|network| network.revision >= slice.revision)
            {
                return;
            }
            guard.dashboard_network_slice = Some(Arc::new(slice));
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    pub(crate) async fn materialize_dashboard_terminal_slice(
        &self,
        slice: DashboardTerminalProjectionSlice,
    ) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            if guard
                .dashboard_terminal_slice
                .as_ref()
                .is_some_and(|terminal| terminal.revision >= slice.revision)
            {
                return;
            }
            guard.dashboard_terminal_slice = Some(Arc::new(slice));
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            mark_dashboard_terminal_window_rebase_topics(&mut guard);
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    async fn materialize_dashboard_parallel_work(&self) {
        let (pending, current, network, terminal) = {
            let mut guard = self.state.lock().await;
            for cached in guard.topics.values_mut() {
                if matches!(
                    &cached.dashboard_materializer,
                    Some(DashboardTopicMaterializer::ParallelWork { .. })
                ) {
                    cached.parallel_work_refresh_scheduled = false;
                }
            }
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let pending = collect_pending_dashboard_topic_materializations(&mut guard);
            (pending, current, network, terminal)
        };
        self.materialize_pending_dashboard_topics(pending, current, network, terminal)
            .await;
    }

    async fn materialize_pending_dashboard_topics(
        &self,
        pending: Vec<PendingDashboardTopicMaterialization>,
        current: Option<Arc<DashboardCurrentProjectionSlice>>,
        network: Option<Arc<DashboardNetworkProjectionSlice>>,
        terminal: Option<Arc<DashboardTerminalProjectionSlice>>,
    ) {
        for pending in pending {
            let serialized_payload = match pending.materializer.serialize(
                current.as_deref(),
                network.as_deref(),
                terminal.as_deref(),
            ) {
                Ok(payload) => payload,
                Err(err) => {
                    warn!(
                        ?err,
                        topic = pending.topic_name,
                        "failed to materialize dashboard topic frame"
                    );
                    continue;
                }
            };
            if let Err(err) = self
                .commit_dashboard_materialized_frame(pending, serialized_payload)
                .await
            {
                warn!(?err, "failed to commit dashboard topic frame");
            }
        }
    }

    async fn commit_dashboard_materialized_frame(
        &self,
        pending: PendingDashboardTopicMaterialization,
        serialized_payload: Vec<u8>,
    ) -> Result<(), ApiError> {
        let dispatch = {
            let mut guard = self.state.lock().await;
            let current = guard.dashboard_current_slice.clone();
            let network = guard.dashboard_network_slice.clone();
            let terminal = guard.dashboard_terminal_slice.clone();
            let Some(cached) = guard.topics.get_mut(&pending.topic_key) else {
                return Ok(());
            };
            let expected_revision =
                cached
                    .dashboard_materializer
                    .as_ref()
                    .and_then(|materializer| {
                        materializer.revision(
                            cached.dashboard_base_revision,
                            current.as_deref(),
                            network.as_deref(),
                            terminal.as_deref(),
                        )
                    });
            if cached.dashboard_base_revision != pending.revision.base_revision
                || expected_revision != Some(pending.revision)
                || cached.dashboard_materialized_revision == Some(pending.revision)
            {
                return Ok(());
            }
            if cached.snapshot_frame.payload_bytes.as_ref() == serialized_payload.as_slice() {
                cached.dashboard_materialized_revision = Some(pending.revision);
                self.dashboard_topology_counters
                    .record_frame_reused(pending.topic_name);
                return Ok(());
            }

            let next_cursor = cached.cursor.saturating_add(1);
            self.dashboard_topology_counters
                .record_materialization(pending.topic_name, false);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                pending.topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.snapshot_frame = frame.clone();
            cached.dashboard_materialized_revision = Some(pending.revision);
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent { frame }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }

    async fn handle_runtime_mutation_batch(
        &self,
        state: Arc<AppState>,
        mutations: Vec<SequencedRuntimeMutation>,
    ) {
        let received_count = mutations.len();
        let mutations = coalesce_runtime_mutations(mutations);
        if mutations.is_empty() {
            return;
        }
        self.runtime_mutation_bus
            .record_router_batch(received_count, mutations.len());
        self.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
            .await;

        for mutation in &mutations {
            match &mutation.mutation {
                RuntimeMutation::PromptCacheBindingChanged { prompt_cache_key } => {
                    if let Err(err) = self
                        .apply_prompt_cache_binding_projection(state.clone(), prompt_cache_key)
                        .await
                    {
                        warn!(
                            ?err,
                            prompt_cache_key,
                            "failed to apply bounded prompt cache binding projection"
                        );
                    }
                }
                RuntimeMutation::StickyRouteChanged {
                    sticky_key,
                    previous_upstream_account_id,
                    upstream_account_id,
                } => {
                    if let Err(err) = self
                        .apply_prompt_cache_sticky_route_projection(
                            state.clone(),
                            sticky_key,
                            *previous_upstream_account_id,
                            *upstream_account_id,
                        )
                        .await
                    {
                        warn!(
                            ?err,
                            sticky_key, "failed to apply prompt cache sticky route projection"
                        );
                    }
                }
                RuntimeMutation::Invocation(_)
                | RuntimeMutation::AttemptChanged { .. }
                | RuntimeMutation::ModelRoutingChanged => {}
            }
        }

        // The dependency index contains only active selections. Disconnection removes a topic
        // from the index and leaves its retained frame dirty, so the router never scans retained
        // caches or clones a runtime event into the hot path.
        let affected = {
            let guard = self.state.lock().await;
            Self::collect_runtime_topic_work(&guard, &mutations)
        };
        self.runtime_mutation_bus.record_topic_work(affected.len());

        for work in affected {
            // Dashboard activity, summary, and network are materialized by their dedicated
            // runtime projection slices. A generic mutation must never send them back through
            // the DB-backed topic builder.
            if work.topic.uses_summary_live_overlay()
                || work.topic.uses_dashboard_activity_live_overlay()
                || work.topic.uses_timeseries_live_projection()
                || work.topic.uses_dashboard_network_live_snapshot()
            {
                continue;
            }
            if work.topic.uses_parallel_work_live_projection() {
                if let Err(err) = self
                    .schedule_parallel_work_topic_projection(
                        state.clone(),
                        work.topic.clone(),
                        &mutations,
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %work.topic.name(),
                        "failed to schedule parallel-work runtime projection"
                    );
                }
                continue;
            }
            if work.topic.uses_summary_topic_refresh() && work.terminal_event_count > 0 {
                if let Err(err) = self
                    .schedule_summary_topic_refresh(
                        state.clone(),
                        work.topic.clone(),
                        work.terminal_event_count,
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %work.topic.name(),
                        "failed to schedule summary topic refresh"
                    );
                }
                continue;
            }
            if work.topic.uses_conversation_overview_refresh() && work.includes_invocation_mutation
            {
                if let Err(err) = self
                    .schedule_conversation_overview_topic_refresh(state.clone(), work.topic.clone())
                    .await
                {
                    warn!(
                        ?err,
                        topic = %work.topic.name(),
                        "failed to schedule conversation overview topic refresh"
                    );
                }
                continue;
            }
            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), work.topic.clone(), true)
                .await
            {
                warn!(
                    ?err,
                    topic = %work.topic.name(),
                    "failed to refresh subscription topic after typed runtime mutation"
                );
            }
        }
    }

    async fn mark_runtime_mutation_gap_and_recover(
        &self,
        state: Arc<AppState>,
        skipped: u64,
        reason: &'static str,
    ) {
        let (active_topic_count, recovery_scheduled) = {
            let mut guard = self.state.lock().await;
            guard.runtime_topic_recovery_generation =
                guard.runtime_topic_recovery_generation.saturating_add(1);
            let recovery_generation = guard.runtime_topic_recovery_generation;
            let active_topic_keys = guard
                .active_topics
                .iter()
                .filter(|(topic_key, _)| {
                    guard
                        .active_subscribers
                        .get(*topic_key)
                        .copied()
                        .unwrap_or_default()
                        > 0
                })
                .map(|(topic_key, _)| topic_key.clone())
                .collect::<Vec<_>>();
            for topic_key in &active_topic_keys {
                let Some(cached) = guard.topics.get_mut(topic_key) else {
                    continue;
                };
                cached.dirty = true;
                cached.refresh_scheduled = false;
                cached.latest_live_snapshot = None;
                cached.continuity_reset_cursor = Some(cached.cursor);
                cached.runtime_topic_recovery_generation = recovery_generation;
                cached.runtime_topic_recovery_retry_at = None;
                if matches!(
                    cached.topic,
                    SubscriptionTopic::PromptCacheWindow { .. }
                        | SubscriptionTopic::PromptCacheStickyWindow { .. }
                        | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                ) {
                    // Prompt Cache keeps its last-good frame. Its server-push reconciler performs
                    // the bounded cold rebuild later instead of doing a full window build from
                    // this cursor-gap handler.
                    cached.prompt_cache_pending_records.clear();
                    cached.prompt_cache_refresh_scheduled = false;
                    cached.prompt_cache_reconcile_required = true;
                    cached.prompt_cache_pressure_deferred = false;
                }
            }
            let recovery_scheduled = Self::enqueue_runtime_topic_recovery_locked(&mut guard);
            (active_topic_keys.len(), recovery_scheduled)
        };
        warn!(
            skipped,
            reason,
            recovery = "dirty_last_good",
            active_topic_count,
            "runtime mutation cursor continuity lost; scheduling bounded topic recovery"
        );
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(state).await;
            });
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    async fn schedule_dirty_topic_recovery(&self, state: Arc<AppState>, topic: SubscriptionTopic) {
        if matches!(
            &topic,
            SubscriptionTopic::PromptCacheWindow { .. }
                | SubscriptionTopic::PromptCacheStickyWindow { .. }
                | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
        ) {
            if self
                .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                .await
            {
                Self::spawn_prompt_cache_topic_reconcile(state, topic);
            }
            return;
        }

        let recovery_scheduled = {
            let mut guard = self.state.lock().await;
            Self::enqueue_runtime_topic_recovery_locked(&mut guard)
        };
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(state).await;
            });
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    fn next_runtime_topic_recovery_retry_delay_locked(
        guard: &SubscriptionHubState,
    ) -> Option<Duration> {
        let now = Instant::now();
        guard
            .active_topics
            .iter()
            .filter_map(|(topic_key, topic)| {
                if guard
                    .active_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
                    || matches!(
                        topic,
                        SubscriptionTopic::PromptCacheWindow { .. }
                            | SubscriptionTopic::PromptCacheStickyWindow { .. }
                            | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    )
                {
                    return None;
                }
                guard.topics.get(topic_key).and_then(|cached| {
                    cached
                        .dirty
                        .then_some(cached.runtime_topic_recovery_retry_at)?
                })
            })
            .filter(|retry_at| *retry_at > now)
            .map(|retry_at| retry_at.duration_since(now))
            .min()
    }

    fn enqueue_runtime_topic_recovery_locked(guard: &mut SubscriptionHubState) -> bool {
        let active_topic_keys = guard
            .active_topics
            .keys()
            .filter(|topic_key| {
                guard
                    .active_subscribers
                    .get(*topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0
            })
            .cloned()
            .collect::<Vec<_>>();
        for topic_key in active_topic_keys {
            if guard.runtime_topic_recovery_queue.len() >= RUNTIME_TOPIC_RECOVERY_QUEUE_CAPACITY {
                break;
            }
            if guard.runtime_topic_recovery_queued.contains(&topic_key) {
                continue;
            }
            let Some(cached) = guard.topics.get(&topic_key) else {
                continue;
            };
            if !cached.dirty
                || cached
                    .runtime_topic_recovery_retry_at
                    .is_some_and(|retry_at| retry_at > Instant::now())
            {
                continue;
            }
            if matches!(
                cached.topic,
                SubscriptionTopic::PromptCacheWindow { .. }
                    | SubscriptionTopic::PromptCacheStickyWindow { .. }
                    | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
            ) {
                continue;
            }
            let recovery_generation = cached.runtime_topic_recovery_generation;
            guard
                .runtime_topic_recovery_queued
                .insert(topic_key.clone());
            guard
                .runtime_topic_recovery_queue
                .push_back((topic_key, recovery_generation));
        }
        if guard.runtime_topic_recovery_running || guard.runtime_topic_recovery_queue.is_empty() {
            return false;
        }
        guard.runtime_topic_recovery_running = true;
        true
    }

    async fn run_runtime_topic_recovery(self: Arc<Self>, state: Arc<AppState>) {
        loop {
            let (topics, retry_delay) = {
                let mut guard = self.state.lock().await;
                let mut topics = Vec::with_capacity(RUNTIME_TOPIC_RECOVERY_BATCH_SIZE);
                while topics.len() < RUNTIME_TOPIC_RECOVERY_BATCH_SIZE {
                    if guard.runtime_topic_recovery_queue.is_empty() {
                        Self::enqueue_runtime_topic_recovery_locked(&mut guard);
                    }
                    let Some((topic_key, recovery_generation)) =
                        guard.runtime_topic_recovery_queue.pop_front()
                    else {
                        break;
                    };
                    guard.runtime_topic_recovery_queued.remove(&topic_key);
                    if guard
                        .active_subscribers
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                        == 0
                    {
                        continue;
                    }
                    let Some(cached) = guard.topics.get(&topic_key) else {
                        continue;
                    };
                    if cached.dirty
                        && cached.runtime_topic_recovery_generation == recovery_generation
                    {
                        topics.push(cached.topic.clone());
                    }
                }
                let retry_delay = (topics.is_empty()
                    && guard.runtime_topic_recovery_queue.is_empty())
                .then(|| Self::next_runtime_topic_recovery_retry_delay_locked(&guard))
                .flatten();
                if topics.is_empty()
                    && guard.runtime_topic_recovery_queue.is_empty()
                    && retry_delay.is_none()
                {
                    guard.runtime_topic_recovery_running = false;
                }
                (topics, retry_delay)
            };
            if topics.is_empty() {
                if let Some(delay) = retry_delay {
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.runtime_topic_recovery_notify.notified() => {}
                    }
                    continue;
                }
                return;
            }
            for topic in topics {
                match self
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    Err(err) => {
                        self.defer_runtime_topic_recovery_retry(&topic).await;
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            recovery = "dirty_last_good",
                            "bounded runtime mutation recovery retained last-good topic frame"
                        );
                    }
                    Ok(_) if self.parallel_work_reconcile_pending(&topic).await => {
                        self.defer_runtime_topic_recovery_retry(&topic).await;
                    }
                    Ok(_) => {}
                }
            }
            tokio::task::yield_now().await;
        }
    }

    async fn defer_runtime_topic_recovery_retry(&self, topic: &SubscriptionTopic) -> Duration {
        let Ok(topic_key) = topic.cache_key() else {
            return RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF;
        };
        let mut guard = self.state.lock().await;
        let has_pending_mutations = guard
            .parallel_work_prebaseline_mutations
            .get(&topic_key)
            .is_some_and(|mutations| !mutations.is_empty());
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            if has_pending_mutations {
                cached.dirty = true;
            }
            cached.runtime_topic_recovery_retry_at =
                Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
        }
        RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF
    }

    pub(crate) async fn handle_internal_broadcast(
        &self,
        state: Arc<AppState>,
        payload: BroadcastPayload,
    ) {
        match payload {
            BroadcastPayload::DashboardNetworkSlice { slice } => {
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                    self.materialize_dashboard_network_slice(*slice).await;
                } else {
                    self.handle_dashboard_network_slice(state, slice).await;
                }
                return;
            }
            BroadcastPayload::DashboardCurrentSlice { slice } => {
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                    self.materialize_dashboard_current_slice(*slice).await;
                }
                return;
            }
            BroadcastPayload::DashboardActivityLive { .. }
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto =>
            {
                return;
            }
            BroadcastPayload::DashboardTerminalSlice { slice } => {
                if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
                    self.materialize_dashboard_terminal_slice(*slice).await;
                }
                return;
            }
            _ => {}
        }
        let affected = {
            let mut guard = self.state.lock().await;
            let active_subscribers = guard.active_subscribers.clone();
            guard
                .topics
                .values_mut()
                .filter(|cached| cached.topic.is_affected_by(&payload))
                .filter_map(|cached| {
                    let topic_key = cached.topic.cache_key().ok()?;
                    let active = active_subscribers
                        .get(&topic_key)
                        .copied()
                        .unwrap_or_default()
                        > 0;
                    if !active {
                        cached.dirty = true;
                        cached.latest_live_snapshot = None;
                        return None;
                    }
                    if cached.dirty {
                        // Runtime cursor recovery owns this selection. Retain last-good until
                        // its bounded work commits instead of allowing another broadcast path
                        // to publish a partial frame.
                        return None;
                    }
                    if matches!(payload, BroadcastPayload::DashboardActivityLive { .. })
                        && (cached.topic.uses_summary_live_overlay()
                            || cached.topic.uses_dashboard_activity_live_overlay())
                        && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
                    {
                        cached.latest_live_snapshot = Some(snapshot.as_ref().clone());
                    }
                    if matches!(payload, BroadcastPayload::DashboardActivityLive { .. }) {
                        self.dashboard_topology_counters
                            .record_business_payload(cached.topic.name());
                    }
                    Some(cached.clone())
                })
                .collect::<Vec<_>>()
        };

        for cached in affected {
            if cached.topic.uses_summary_live_overlay()
                && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
            {
                if let Err(err) = self
                    .apply_summary_live_overlay(&cached.topic, snapshot.as_ref().clone())
                    .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to apply summary live overlay"
                    );
                }
                continue;
            }

            if cached.topic.uses_dashboard_activity_live_overlay()
                && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
            {
                if let Err(err) = self
                    .apply_dashboard_activity_live_overlay(
                        state.clone(),
                        &cached.topic,
                        snapshot.as_ref().clone(),
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to apply dashboard activity live overlay"
                    );
                }
                continue;
            }

            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), cached.topic.clone(), true)
                .await
            {
                warn!(
                    ?err,
                    topic = %cached.topic.name(),
                    "failed to refresh subscription topic"
                );
            }
        }
    }

    async fn handle_dashboard_network_slice(
        &self,
        state: Arc<AppState>,
        slice: Box<DashboardNetworkProjectionSlice>,
    ) {
        let topics = {
            let guard = self.state.lock().await;
            guard
                .topics
                .values()
                .filter(|cached| {
                    matches!(
                        cached.topic,
                        SubscriptionTopic::DashboardActivityCurrent { .. }
                            | SubscriptionTopic::DashboardNetworkTimeseriesWindow { .. }
                    )
                })
                .filter(|cached| {
                    cached
                        .topic
                        .cache_key()
                        .ok()
                        .and_then(|key| guard.active_subscribers.get(&key).copied())
                        .unwrap_or_default()
                        > 0
                })
                .map(|cached| cached.topic.clone())
                .collect::<Vec<_>>()
        };

        for topic in topics {
            self.dashboard_topology_counters
                .record_business_payload(topic.name());
            let result = match &topic {
                SubscriptionTopic::DashboardActivityCurrent { .. } => {
                    if let Some(live) = state
                        .proxy_runtime_invocations
                        .legacy_live_snapshot_for_network(slice.as_ref())
                    {
                        self.apply_dashboard_activity_live_overlay(state.clone(), &topic, live)
                            .await
                    } else {
                        Ok(())
                    }
                }
                SubscriptionTopic::DashboardNetworkTimeseriesWindow { .. } => {
                    self.apply_dashboard_network_slice_to_timeseries(&topic, slice.as_ref())
                        .await
                }
                _ => Ok(()),
            };
            if let Err(err) = result {
                warn!(?err, topic = %topic.name(), "failed to apply dashboard network slice");
            }
        }
    }
}

fn buffer_parallel_work_prebaseline_mutations(
    state: &mut SubscriptionHubState,
    topic_key: &str,
    mutations: &[SequencedRuntimeMutation],
) {
    let pending_is_empty = {
        let pending = state
            .parallel_work_prebaseline_mutations
            .entry(topic_key.to_string())
            .or_default();
        for mutation in mutations {
            let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                continue;
            };
            let identity = format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            );
            if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
                pending.remove(&identity);
            } else {
                pending.insert(identity, mutation.clone());
            }
        }
        pending.is_empty()
    };
    if pending_is_empty {
        state.parallel_work_prebaseline_mutations.remove(topic_key);
    }
}

fn remove_parallel_work_prebaseline_mutations(
    state: &mut SubscriptionHubState,
    topic_key: &str,
    mutations: &[SequencedRuntimeMutation],
) {
    let Some(pending) = state.parallel_work_prebaseline_mutations.get_mut(topic_key) else {
        return;
    };
    for mutation in mutations {
        let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
            continue;
        };
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            pending.remove(&format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            ));
        }
    }
    if pending.is_empty() {
        state.parallel_work_prebaseline_mutations.remove(topic_key);
    }
}

fn rolling_dashboard_window_requires_rebase(
    base_start: Option<DateTime<Utc>>,
    current_start: Option<DateTime<Utc>>,
) -> bool {
    match (base_start, current_start) {
        (Some(base_start), Some(current_start)) => {
            current_start < base_start
                || current_start - base_start
                    >= ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64)
        }
        (base_start, current_start) => base_start != current_start,
    }
}

fn mark_dashboard_terminal_window_rebase_topics(guard: &mut SubscriptionHubState) {
    let active_subscribers = guard.active_subscribers.clone();
    for (topic_key, cached) in &mut guard.topics {
        if cached.dirty
            || !cached
                .dashboard_materializer
                .as_ref()
                .is_some_and(DashboardTopicMaterializer::requires_terminal_window_rebase)
        {
            continue;
        }
        cached.dirty = true;
        cached.refresh_scheduled = active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        cached.latest_live_snapshot = None;
    }
}

fn collect_pending_dashboard_topic_materializations(
    guard: &mut SubscriptionHubState,
) -> Vec<PendingDashboardTopicMaterialization> {
    let active_subscribers = guard.active_subscribers.clone();
    let server_push_subscribers = guard.server_push_subscribers.clone();
    let current = guard.dashboard_current_slice.clone();
    let network = guard.dashboard_network_slice.clone();
    let terminal = guard.dashboard_terminal_slice.clone();
    guard
        .topics
        .iter_mut()
        .filter_map(|(topic_key, cached)| {
            let active = active_subscribers
                .get(topic_key)
                .copied()
                .unwrap_or_default()
                > 0
                || server_push_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0;
            if !active {
                cached.dirty = true;
                return None;
            }
            if cached.dirty {
                return None;
            }
            let materializer = cached.dashboard_materializer.as_ref()?;
            let revision = materializer.revision(
                cached.dashboard_base_revision,
                current.as_deref(),
                network.as_deref(),
                terminal.as_deref(),
            )?;
            (cached.dashboard_materialized_revision != Some(revision)).then(|| {
                PendingDashboardTopicMaterialization {
                    topic_key: topic_key.clone(),
                    topic_name: cached.topic.name(),
                    revision,
                    materializer: materializer.clone(),
                }
            })
        })
        .collect()
}

impl Default for SubscriptionHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SubscriptionHub {
    async fn mark_topic_dirty(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.dirty = true;
            cached.refresh_scheduled = false;
            cached.conversation_overview_refresh_scheduled = false;
            cached.conversation_overview_refresh_in_flight = false;
            cached.conversation_overview_refresh_pending = false;
            cached.latest_live_snapshot = None;
        }
    }

    async fn mark_prompt_cache_topic_dirty_and_schedule_reconcile(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.prompt_cache_refresh_scheduled = false;
        cached.prompt_cache_reconcile_required = true;
        cached.prompt_cache_pressure_deferred = false;
        if active && !cached.prompt_cache_reconcile_scheduled {
            cached.prompt_cache_reconcile_scheduled = true;
            return true;
        }
        false
    }

    async fn prompt_cache_topic_reconcile_delay(
        &self,
        topic: &SubscriptionTopic,
    ) -> Option<Duration> {
        let topic_key = topic.cache_key().ok()?;
        let guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let cached = guard.topics.get(&topic_key)?;
        if !active || (!cached.dirty && !cached.prompt_cache_reconcile_required) {
            return None;
        }
        Some(
            cached
                .prompt_cache_baseline_at
                .map(|baseline_at| {
                    PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL.saturating_sub(baseline_at.elapsed())
                })
                .unwrap_or_default(),
        )
    }

    async fn begin_prompt_cache_topic_reconcile(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if !active
            || cached.prompt_cache_reconcile_scheduled
            || (!cached.dirty && !cached.prompt_cache_reconcile_required)
        {
            return false;
        }
        cached.prompt_cache_reconcile_scheduled = true;
        true
    }

    async fn begin_conversation_overview_topic_refresh(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if !cached.conversation_overview_refresh_scheduled {
            return false;
        }
        cached.conversation_overview_refresh_in_flight = true;
        true
    }

    async fn finish_conversation_overview_topic_refresh(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        let rerun = cached.conversation_overview_refresh_pending;
        cached.conversation_overview_refresh_scheduled = false;
        cached.conversation_overview_refresh_in_flight = false;
        cached.conversation_overview_refresh_pending = false;
        rerun
    }

    async fn rearm_conversation_overview_topic_refresh(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if active == 0 {
            cached.dirty = true;
            return false;
        }
        cached.conversation_overview_refresh_scheduled = true;
        true
    }

    async fn schedule_conversation_overview_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let delay = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 {
                cached.dirty = true;
                return Ok(());
            }
            if cached.conversation_overview_refresh_scheduled {
                if cached.conversation_overview_refresh_in_flight {
                    cached.conversation_overview_refresh_pending = true;
                }
                return Ok(());
            }
            cached.conversation_overview_refresh_scheduled = true;
            CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE
        };
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            let mut delay = delay;
            loop {
                tokio::time::sleep(delay).await;
                if !hub.begin_conversation_overview_topic_refresh(&topic).await {
                    return;
                }
                if !hub.has_active_topic_key(&topic_key).await {
                    tracing::debug!(
                        topic = %topic.name(),
                        refresh_outcome = "marked_dirty",
                        "skipping deferred conversation overview refresh without owner subscribers"
                    );
                    hub.finish_conversation_overview_topic_refresh(&topic).await;
                    hub.mark_topic_dirty(&topic).await;
                    return;
                }
                let result = hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await;
                let rerun = hub.finish_conversation_overview_topic_refresh(&topic).await;
                match result {
                    Ok(Some(_)) | Ok(None) => {}
                    Err(err) => {
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            refresh_outcome = "retained_last_good",
                            "conversation overview topic refresh failed"
                        );
                    }
                }
                if !rerun || !hub.rearm_conversation_overview_topic_refresh(&topic).await {
                    return;
                }
                delay = CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE;
            }
        });
        Ok(())
    }

    async fn schedule_parallel_work_topic_projection(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        mutations: &[SequencedRuntimeMutation],
    ) -> Result<(), ApiError> {
        enum ParallelWorkSchedule {
            None,
            Materialize(Instant),
            Reconcile,
        }

        let topic_key = topic.cache_key()?;
        loop {
            let materializer = {
                let mut guard = self.state.lock().await;
                let active = guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default();
                if active == 0 {
                    if let Some(cached) = guard.topics.get_mut(&topic_key) {
                        cached.dirty = true;
                    }
                    return Ok(());
                }
                if !guard.topics.contains_key(&topic_key) {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                if guard.topics[&topic_key].dirty {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                let cached = guard
                    .topics
                    .get_mut(&topic_key)
                    .expect("cached topic exists");
                let Some(DashboardTopicMaterializer::ParallelWork { base }) =
                    cached.dashboard_materializer.as_ref()
                else {
                    return Ok(());
                };
                base.clone()
            };

            let outcome = {
                let mut base = materializer
                    .lock()
                    .expect("parallel-work materializer state lock");
                let mut outcome = ParallelWorkMutationOutcome::default();
                for mutation in mutations {
                    let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                        continue;
                    };
                    let mutation_outcome = base.apply_runtime_mutation(mutation);
                    outcome.changed |= mutation_outcome.changed;
                    outcome.needs_account_reconcile |= mutation_outcome.needs_account_reconcile;
                }
                outcome
            };

            let schedule = {
                let mut guard = self.state.lock().await;
                let active = guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default();
                if active == 0 {
                    if let Some(cached) = guard.topics.get_mut(&topic_key) {
                        cached.dirty = true;
                    }
                    return Ok(());
                }
                remove_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                if !guard.topics.contains_key(&topic_key) {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                if guard.topics[&topic_key].dirty {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                    return Ok(());
                }
                if outcome.needs_account_reconcile {
                    buffer_parallel_work_prebaseline_mutations(&mut guard, &topic_key, mutations);
                }
                let cached = guard
                    .topics
                    .get_mut(&topic_key)
                    .expect("cached topic exists");
                if !matches!(
                    cached.dashboard_materializer.as_ref(),
                    Some(DashboardTopicMaterializer::ParallelWork { base }) if Arc::ptr_eq(base, &materializer)
                ) {
                    None
                } else if outcome.needs_account_reconcile {
                    if cached.parallel_work_refresh_scheduled {
                        Some(ParallelWorkSchedule::None)
                    } else {
                        cached.dirty = true;
                        cached.parallel_work_refresh_scheduled = true;
                        Some(ParallelWorkSchedule::Reconcile)
                    }
                } else if !outcome.changed || cached.parallel_work_refresh_scheduled {
                    Some(ParallelWorkSchedule::None)
                } else {
                    cached.parallel_work_refresh_scheduled = true;
                    Some(ParallelWorkSchedule::Materialize(Instant::now()))
                }
            };
            let Some(schedule) = schedule else {
                tokio::task::yield_now().await;
                continue;
            };
            match schedule {
                ParallelWorkSchedule::None => {}
                ParallelWorkSchedule::Materialize(scheduled_at) => {
                    let hub = state.subscription_hub.clone();
                    tokio::spawn(async move {
                        let deadline = scheduled_at + PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE;
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                        if Instant::now().saturating_duration_since(deadline)
                            > PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE
                        {
                            hub.dashboard_topology_counters
                                .record_cadence_miss("stats.parallel-work.current");
                        }
                        hub.materialize_dashboard_parallel_work().await;
                    });
                }
                ParallelWorkSchedule::Reconcile => {
                    // Account gaps are recovered from a dirty last-good frame, not by the
                    // in-memory hot materializer. Recovery health reports that state directly;
                    // this background rebaseline delay is therefore not a hot cadence miss.
                    let hub = state.subscription_hub.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE).await;
                        let result = hub
                            .refresh_topic_if_active(state.clone(), topic.clone(), true)
                            .await;
                        if let Err(err) = result {
                            warn!(
                                ?err,
                                topic = %topic.name(),
                                "failed to reconcile account-scoped parallel-work projection"
                            );
                            hub.schedule_parallel_work_reconcile_retry(state, &topic)
                                .await;
                        } else if hub.parallel_work_reconcile_pending(&topic).await {
                            hub.schedule_parallel_work_reconcile_retry(state, &topic)
                                .await;
                        }
                    });
                }
            }
            return Ok(());
        }
    }

    async fn schedule_parallel_work_reconcile_retry(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let recovery_scheduled = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                > 0;
            let has_pending_mutations = guard
                .parallel_work_prebaseline_mutations
                .get(&topic_key)
                .is_some_and(|mutations| !mutations.is_empty());
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            cached.parallel_work_refresh_scheduled = false;
            if has_pending_mutations {
                cached.dirty = true;
            }
            if !active {
                return;
            }
            cached.runtime_topic_recovery_retry_at =
                Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
            if guard.runtime_topic_recovery_running {
                false
            } else {
                guard.runtime_topic_recovery_running = true;
                true
            }
        };
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(state).await;
            });
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    async fn parallel_work_reconcile_pending(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        self.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .get(&topic_key)
            .is_some_and(|mutations| !mutations.is_empty())
    }

    async fn schedule_prompt_cache_topic_projection(
        &self,
        state: Arc<AppState>,
        mutations: &[SequencedRuntimeMutation],
    ) {
        // The active dependency lookup comes before the compact preview lookup below. Inactive
        // Prompt Cache topics therefore never allocate preview data or read runtime state.
        let active_topic_keys = {
            let guard = self.state.lock().await;
            let mut topic_keys = Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::PromptCacheProjection,
            );
            topic_keys.extend(Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::DashboardWorkingConversationsProjection,
            ));
            topic_keys
                .into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };
        if active_topic_keys.is_empty() {
            return;
        }

        let mut records = Vec::new();
        let mut reconcile_required = false;
        let mut working_key_hydration_keys = BTreeSet::new();
        let mut working_reconcile_required = false;
        for mutation in mutations {
            let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                continue;
            };
            if mutation.prompt_cache_key.is_none() && mutation.sticky_key.is_none() {
                continue;
            }
            let runtime_projection =
                (mutation.kind != RuntimeMutationKind::RuntimeRemoved).then(|| {
                    state
                        .proxy_runtime_invocations
                        .prompt_cache_projection_by_identity(
                            &mutation.identity.invoke_id,
                            &mutation.identity.occurred_at,
                        )
                });
            let runtime_projection = runtime_projection.flatten();
            match PromptCacheTopicDelta::from_runtime_mutation(
                mutation,
                runtime_projection.as_ref(),
            ) {
                Ok(Some(record)) => records.push(record),
                Ok(None) => {
                    reconcile_required = true;
                    // A terminal can leave the runtime store immediately after P1 assigns its
                    // durable row id. Working conversations can recover that one durable key
                    // exactly; treating it as a generic gap would needlessly rebuild the whole
                    // active window.
                    if mutation.is_terminal
                        && mutation.row_id.is_some_and(|row_id| row_id > 0)
                        && let Some(prompt_cache_key) = mutation.prompt_cache_key.as_deref()
                    {
                        working_key_hydration_keys.insert(prompt_cache_key.to_string());
                    } else {
                        working_reconcile_required = true;
                    }
                }
                Err(err) => {
                    reconcile_required = true;
                    working_reconcile_required = true;
                    warn!(?err, "failed to build active prompt cache topic delta");
                }
            }
        }
        if records.is_empty() && !reconcile_required {
            return;
        }

        let (scheduled, key_hydrations, reconciles) = {
            let mut guard = self.state.lock().await;
            let mut scheduled = Vec::new();
            let mut key_hydrations = Vec::new();
            let mut reconciles = Vec::new();
            for topic_key in active_topic_keys {
                let Some(topic) = guard.active_topics.get(&topic_key).cloned() else {
                    continue;
                };
                let Some(cached) = guard.topics.get_mut(&topic_key) else {
                    if !records.is_empty() {
                        let pending = guard
                            .prompt_cache_prebaseline_records
                            .entry(topic_key.clone())
                            .or_default();
                        for record in &records {
                            pending.insert(record.identity.clone(), record.clone());
                        }
                    }
                    if matches!(
                        topic,
                        SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    ) && !working_key_hydration_keys.is_empty()
                    {
                        guard
                            .prompt_cache_prebaseline_key_hydrations
                            .entry(topic_key)
                            .or_default()
                            .extend(working_key_hydration_keys.iter().cloned());
                    }
                    continue;
                };
                let is_working_conversations = matches!(
                    topic,
                    SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                );
                let requires_bounded_reconcile = if is_working_conversations {
                    working_reconcile_required || cached.dirty
                } else {
                    reconcile_required || cached.dirty
                };
                if requires_bounded_reconcile {
                    cached.dirty = true;
                    cached.prompt_cache_reconcile_required = true;
                    if !cached.prompt_cache_reconcile_scheduled {
                        cached.prompt_cache_reconcile_scheduled = true;
                        reconciles.push(topic);
                    }
                    // A gap means the retained payload may be missing an earlier mutation.
                    // Do not append a later delta to that stale frame; one active selection is
                    // rebuilt asynchronously from its bounded cold source instead.
                    continue;
                }
                if is_working_conversations && !working_key_hydration_keys.is_empty() {
                    cached
                        .prompt_cache_pending_key_hydrations
                        .extend(working_key_hydration_keys.iter().cloned());
                    if !cached.prompt_cache_key_hydration_scheduled {
                        cached.prompt_cache_key_hydration_scheduled = true;
                        key_hydrations.push(topic.clone());
                    }
                }
                if records.is_empty() {
                    continue;
                }
                let before = cached.prompt_cache_pending_records.len();
                for record in &records {
                    cached
                        .prompt_cache_pending_records
                        .insert(record.identity.clone(), record.clone());
                }
                cached.prompt_cache_coalesced_event_count =
                    cached.prompt_cache_coalesced_event_count.saturating_add(
                        records.len().saturating_sub(
                            cached
                                .prompt_cache_pending_records
                                .len()
                                .saturating_sub(before),
                        ) as u64,
                    );
                if !cached.prompt_cache_refresh_scheduled {
                    cached.prompt_cache_refresh_scheduled = true;
                    scheduled.push(topic);
                }
            }
            (scheduled, key_hydrations, reconciles)
        };

        for topic in scheduled {
            let hub = state.subscription_hub.clone();
            let state = state.clone();
            let deadline = Instant::now() + PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE;
            tokio::spawn(async move {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                if Instant::now().saturating_duration_since(deadline)
                    > PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE
                {
                    hub.dashboard_topology_counters
                        .record_cadence_miss(topic.name());
                }
                if let Err(err) = hub
                    .materialize_prompt_cache_topic(state.clone(), &topic)
                    .await
                {
                    warn!(
                        ?err,
                        topic = topic.name(),
                        response_source = "last_good",
                        "prompt cache in-memory topic materialization failed"
                    );
                    if hub
                        .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                        .await
                    {
                        SubscriptionHub::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
                    }
                }
            });
        }
        for topic in key_hydrations {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        for topic in reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
    }

    fn spawn_prompt_cache_topic_reconcile(state: Arc<AppState>, topic: SubscriptionTopic) {
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            loop {
                let Some(delay) = hub.prompt_cache_topic_reconcile_delay(&topic).await else {
                    hub.finish_prompt_cache_topic_reconcile(&topic).await;
                    return;
                };
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {}
                    _ = state.shutdown.cancelled() => {
                        hub.finish_prompt_cache_topic_reconcile(&topic).await;
                        return;
                    }
                }
                let Ok(topic_key) = topic.cache_key() else {
                    hub.finish_prompt_cache_topic_reconcile(&topic).await;
                    return;
                };
                if !hub.has_active_topic_key(&topic_key).await {
                    hub.finish_prompt_cache_topic_reconcile(&topic).await;
                    return;
                }
                let gate = crate::db_pressure::global_db_pressure_gate();
                let observed_eligibility = gate.eligibility_generation();
                match gate.try_begin_background("prompt_cache_topic_reconcile") {
                    Ok(_permit) => {
                        let result = hub
                            .refresh_topic_if_active(state.clone(), topic.clone(), true)
                            .await;
                        hub.finish_prompt_cache_topic_reconcile(&topic).await;
                        if let Err(err) = result {
                            warn!(
                                ?err,
                                topic = topic.name(),
                                response_source = "last_good",
                                "bounded prompt cache topic reconcile failed"
                            );
                            hub.mark_topic_dirty(&topic).await;
                            hub.set_prompt_cache_pressure_deferred(&topic, false).await;
                        }
                        return;
                    }
                    Err(reason) => {
                        hub.set_prompt_cache_pressure_deferred(&topic, true).await;
                        tracing::debug!(
                            topic = %topic.name(),
                            reconcile_outcome = "pressure_deferred",
                            defer_reason = %reason,
                            "prompt cache topic reconcile deferred"
                        );
                        tokio::select! {
                            _ = wait_for_prompt_cache_reconcile_eligibility(
                                gate,
                                observed_eligibility,
                                reason,
                            ) => {}
                            _ = state.shutdown.cancelled() => {
                                hub.finish_prompt_cache_topic_reconcile(&topic).await;
                                return;
                            }
                        }
                    }
                }
            }
        });
    }

    async fn finish_prompt_cache_topic_reconcile(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_reconcile_scheduled = false;
        }
    }

    async fn set_prompt_cache_pressure_deferred(
        &self,
        topic: &SubscriptionTopic,
        pressure_deferred: bool,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_pressure_deferred = pressure_deferred;
        }
    }

    fn spawn_prompt_cache_topic_materialization(state: Arc<AppState>, topic: SubscriptionTopic) {
        let hub = state.subscription_hub.clone();
        let deadline = Instant::now() + PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE;
        tokio::spawn(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            if Instant::now().saturating_duration_since(deadline)
                > PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE
            {
                hub.dashboard_topology_counters
                    .record_cadence_miss(topic.name());
            }
            if let Err(err) = hub
                .materialize_prompt_cache_topic(state.clone(), &topic)
                .await
            {
                warn!(
                    ?err,
                    topic = topic.name(),
                    response_source = "last_good",
                    "prompt cache in-memory topic materialization failed"
                );
                if hub
                    .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                    .await
                {
                    SubscriptionHub::spawn_prompt_cache_topic_reconcile(state, topic);
                }
            }
        });
    }

    fn spawn_dashboard_working_conversation_key_hydration(
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) {
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            loop {
                let gate = crate::db_pressure::global_db_pressure_gate();
                let observed_eligibility = gate.eligibility_generation();
                match gate.try_begin_background("dashboard_working_conversation_key_hydrate") {
                    Ok(_permit) => {
                        if let Err(err) = hub
                            .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
                            .await
                        {
                            warn!(
                                ?err,
                                topic = topic.name(),
                                response_source = "last_good",
                                "bounded dashboard working conversation hydrate failed"
                            );
                            hub.finish_dashboard_working_conversation_key_hydration(&topic)
                                .await;
                            if hub
                                .mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                                .await
                            {
                                SubscriptionHub::spawn_prompt_cache_topic_reconcile(
                                    state.clone(),
                                    topic,
                                );
                            }
                        }
                        return;
                    }
                    Err(reason) => {
                        hub.set_prompt_cache_pressure_deferred(&topic, true).await;
                        tracing::debug!(
                            topic = %topic.name(),
                            hydrate_outcome = "pressure_deferred",
                            defer_reason = %reason,
                            "dashboard working conversation key hydrate deferred"
                        );
                        tokio::select! {
                            _ = wait_for_prompt_cache_reconcile_eligibility(
                                gate,
                                observed_eligibility,
                                reason,
                            ) => {}
                            _ = state.shutdown.cancelled() => {
                                hub.finish_dashboard_working_conversation_key_hydration(&topic)
                                    .await;
                                return;
                            }
                        }
                    }
                }
            }
        });
    }

    async fn finish_dashboard_working_conversation_key_hydration(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
            cached.prompt_cache_key_hydration_scheduled = false;
        }
    }

    async fn hydrate_dashboard_working_conversation_keys(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let Some((
            pending_keys,
            pending_records_at_hydration_start,
            candidate_refill_required,
            page_size,
            recent_invocation_limit,
            blocked_binding_filter,
            visible_keys,
        )) = ({
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 || cached.dirty {
                cached.prompt_cache_key_hydration_scheduled = false;
                None
            } else {
                let Some(DashboardTopicMaterializer::WorkingConversations { state }) =
                    cached.dashboard_materializer.as_ref()
                else {
                    cached.prompt_cache_key_hydration_scheduled = false;
                    return Ok(());
                };
                let working_state = state.clone();
                let state = working_state
                    .lock()
                    .expect("working conversations materializer state lock");
                Some((
                    cached.prompt_cache_pending_key_hydrations.clone(),
                    cached.prompt_cache_pending_records.clone(),
                    cached.prompt_cache_candidate_refill_required,
                    state.page_size,
                    state.recent_invocation_limit,
                    state.blocked_binding_filter.clone(),
                    state.visible_keys(),
                ))
            }
        })
        else {
            return Ok(());
        };

        let range_end = Utc::now();
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let source_scope = resolve_default_source_scope(&state.pool).await?;
        let candidate_keys = if candidate_refill_required {
            query_working_prompt_cache_conversation_candidate_keys(
                state.as_ref(),
                source_scope,
                range_end,
                &range_start_bound,
                page_size as i64,
                blocked_binding_filter.as_ref(),
            )
            .await?
        } else {
            Vec::new()
        };
        let mut hydration_keys = pending_keys;
        hydration_keys.extend(
            candidate_keys
                .into_iter()
                .filter(|key| !visible_keys.contains(key)),
        );
        let hydration_keys = hydration_keys
            .into_iter()
            .take(SUBSCRIPTION_CONVERSATION_OPERATION_LIMIT)
            .collect::<Vec<_>>();
        let mut hydrated = Vec::with_capacity(hydration_keys.len());
        for prompt_cache_key in &hydration_keys {
            let response = hydrate_working_prompt_cache_conversation_for_key(
                state.as_ref(),
                source_scope,
                prompt_cache_key,
                range_end,
                &range_start_bound,
                recent_invocation_limit as i64,
                blocked_binding_filter.as_ref(),
            )
            .await?;
            hydrated.push((prompt_cache_key.clone(), response));
        }
        let hydrated_visible_keys = hydrated
            .iter()
            .filter_map(|(prompt_cache_key, response)| {
                response.as_ref().map(|_| prompt_cache_key.clone())
            })
            .collect::<HashSet<_>>();
        let total_matched = query_working_prompt_cache_conversation_total_matched(
            state.as_ref(),
            source_scope,
            range_end,
            &range_start_bound,
            blocked_binding_filter.as_ref(),
            &hydrated_visible_keys,
        )
        .await?;

        let (dispatch, next_hydration, next_materialization, recovery) = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 || cached.dirty {
                cached.prompt_cache_key_hydration_scheduled = false;
                return Ok(());
            }
            let Some(DashboardTopicMaterializer::WorkingConversations { state: current }) =
                cached.dashboard_materializer.as_ref()
            else {
                cached.prompt_cache_key_hydration_scheduled = false;
                return Ok(());
            };
            let current = current.clone();
            let mut projection = current
                .lock()
                .expect("working conversations materializer state lock");
            let now = Utc::now();
            let changed_pending_keys = prompt_cache_hydration_changed_pending_keys(
                &pending_records_at_hydration_start,
                &cached.prompt_cache_pending_records,
                &hydration_keys,
            );
            let unresolved_eligible_delta = hydrated.iter().any(|(prompt_cache_key, response)| {
                response.is_none()
                    && cached.prompt_cache_pending_records.values().any(|record| {
                        record.prompt_cache_key.as_deref() == Some(prompt_cache_key.as_str())
                            && projection.delta_is_eligible(record, now)
                    })
            });
            if !changed_pending_keys.is_empty() {
                // A newer delta for an already-hydrated key is not represented by this bounded
                // snapshot. Retry that same key immediately instead of making the whole working
                // selection dirty and delaying exact recovery to the 60-second full cadence.
                cached
                    .prompt_cache_pending_key_hydrations
                    .extend(changed_pending_keys);
                cached.prompt_cache_key_hydration_scheduled = true;
                cached.prompt_cache_pressure_deferred = false;
                (None, Some(cached.topic.clone()), None, None)
            } else if unresolved_eligible_delta {
                // A bounded snapshot can race a just-arrived runtime delta or omit an eligible
                // one. Retain the last-good frame and recover instead of clearing a delta that
                // was not proven to be represented by the hydrate result.
                cached.dirty = true;
                cached.prompt_cache_reconcile_required = true;
                cached.prompt_cache_key_hydration_scheduled = false;
                cached.prompt_cache_pressure_deferred = false;
                let recovery = (!cached.prompt_cache_reconcile_scheduled).then(|| {
                    cached.prompt_cache_reconcile_scheduled = true;
                    cached.topic.clone()
                });
                (None, None, None, recovery)
            } else {
                let mut changed = false;
                for (prompt_cache_key, response) in hydrated {
                    // The bounded database/runtime hydrate includes every pending record for
                    // this key, so dropping those records avoids both double-application and a
                    // later full-window reconcile solely for terminal de-duplication bookkeeping.
                    cached.prompt_cache_pending_records.retain(|_, record| {
                        record.prompt_cache_key.as_deref() != Some(prompt_cache_key.as_str())
                    });
                    cached
                        .prompt_cache_pending_key_hydrations
                        .remove(&prompt_cache_key);
                    changed |=
                        projection.replace_hydrated_conversation(&prompt_cache_key, response);
                }
                changed |= projection.set_total_matched(total_matched);
                let desired_visible_count = usize::try_from(total_matched.max(0))
                    .unwrap_or(usize::MAX)
                    .min(projection.page_size);
                cached.prompt_cache_candidate_refill_required =
                    projection.visible_keys().len() < desired_visible_count;
                cached.prompt_cache_bounded_key_hydration_count = cached
                    .prompt_cache_bounded_key_hydration_count
                    .saturating_add(hydration_keys.len() as u64);
                cached.prompt_cache_pressure_deferred = false;
                cached.prompt_cache_key_hydration_scheduled = false;
                let next_hydration = (!cached.prompt_cache_pending_key_hydrations.is_empty()
                    || cached.prompt_cache_candidate_refill_required)
                    .then(|| {
                        cached.prompt_cache_key_hydration_scheduled = true;
                        cached.topic.clone()
                    });
                let next_materialization = (!cached.prompt_cache_pending_records.is_empty()
                    && !cached.prompt_cache_refresh_scheduled)
                    .then(|| {
                        cached.prompt_cache_refresh_scheduled = true;
                        cached.topic.clone()
                    });
                let dispatch = if changed {
                    let next_cursor = cached.cursor.saturating_add(1);
                    self.dashboard_topology_counters
                        .record_materialization(cached.topic.name(), false);
                    let frame = Arc::new(self.serialize_frame(
                        cached.descriptor.clone(),
                        topic_key.clone(),
                        cached.schema_epoch.clone(),
                        next_cursor,
                        projection.serialize()?,
                    )?);
                    let retained_bytes = frame.retained_bytes();
                    cached.cursor = next_cursor;
                    cached.snapshot_frame = frame.clone();
                    cached.snapshot_bytes = frame.payload_bytes.len();
                    cached.replay_events.push_back(ReplayableTopicEvent {
                        frame: frame.clone(),
                        bytes: retained_bytes,
                        emitted_at: Utc::now(),
                    });
                    cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
                    cached.prompt_cache_response_source = "database_bounded_key_hydrate";
                    prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
                    Some(SubscriptionDispatchEvent { frame })
                } else {
                    None
                };
                (dispatch, next_hydration, next_materialization, None)
            }
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        if let Some(topic) = next_hydration {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        if let Some(topic) = next_materialization {
            Self::spawn_prompt_cache_topic_materialization(state.clone(), topic);
        }
        if let Some(topic) = recovery {
            Self::spawn_prompt_cache_topic_reconcile(state, topic);
        }
        Ok(())
    }

    async fn materialize_prompt_cache_topic(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let mut bounded_hydration_topic = None;
        let dispatch = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            cached.prompt_cache_refresh_scheduled = false;
            if active == 0 {
                cached.prompt_cache_pending_records.clear();
                return Ok(());
            }
            let records = std::mem::take(&mut cached.prompt_cache_pending_records)
                .into_values()
                .collect::<Vec<_>>();
            if records.is_empty() {
                return Ok(());
            }
            let working_state = match cached.dashboard_materializer.as_ref() {
                Some(DashboardTopicMaterializer::WorkingConversations { state }) => {
                    Some(state.clone())
                }
                _ => None,
            };
            let is_working_conversations = working_state.is_some();
            let applied = if let Some(working_state) = working_state.as_ref() {
                let update = working_state
                    .lock()
                    .expect("working conversations materializer state lock")
                    .apply_deltas(
                        &records,
                        &mut cached.prompt_cache_applied_terminal_ids,
                        cached.prompt_cache_baseline_row_id,
                    )?;
                match update {
                    WorkingConversationsProjectionUpdate::Unchanged => Some(false),
                    WorkingConversationsProjectionUpdate::Changed => Some(true),
                    WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(keys) => {
                        for record in &records {
                            cached
                                .prompt_cache_pending_records
                                .insert(record.identity.clone(), record.clone());
                        }
                        cached.prompt_cache_pending_key_hydrations.extend(keys);
                        if !cached.prompt_cache_key_hydration_scheduled {
                            cached.prompt_cache_key_hydration_scheduled = true;
                            bounded_hydration_topic = Some(cached.topic.clone());
                        }
                        None
                    }
                    WorkingConversationsProjectionUpdate::NeedsReconcile => {
                        for record in &records {
                            cached
                                .prompt_cache_pending_records
                                .insert(record.identity.clone(), record.clone());
                        }
                        return Err(ApiError::from(anyhow!(
                            "working conversations projection requires bounded reconcile"
                        )));
                    }
                }
            } else {
                match apply_prompt_cache_records_to_payload(
                    topic,
                    &mut cached.snapshot_payload,
                    &records,
                    &mut cached.prompt_cache_applied_terminal_ids,
                    cached.prompt_cache_baseline_row_id,
                ) {
                    Ok(applied) => Some(applied),
                    Err(err) => {
                        for record in &records {
                            cached
                                .prompt_cache_pending_records
                                .insert(record.identity.clone(), record.clone());
                        }
                        return Err(err);
                    }
                }
            };
            if applied != Some(true) {
                None
            } else {
                let next_cursor = cached.cursor.saturating_add(1);
                if is_working_conversations {
                    self.dashboard_topology_counters
                        .record_materialization(topic.name(), false);
                }
                let serialized_payload = if let Some(working_state) = working_state {
                    working_state
                        .lock()
                        .expect("working conversations materializer state lock")
                        .serialize()?
                } else {
                    serde_json::to_vec(&cached.snapshot_payload)?
                };
                let frame = Arc::new(self.serialize_frame(
                    cached.descriptor.clone(),
                    topic_key.clone(),
                    cached.schema_epoch.clone(),
                    next_cursor,
                    serialized_payload,
                )?);
                let retained_bytes = frame.retained_bytes();
                cached.cursor = next_cursor;
                cached.snapshot_frame = frame.clone();
                cached.snapshot_bytes = frame.payload_bytes.len();
                cached.replay_events.push_back(ReplayableTopicEvent {
                    frame: frame.clone(),
                    bytes: retained_bytes,
                    emitted_at: Utc::now(),
                });
                cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
                cached.prompt_cache_response_source = "memory";
                prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
                tracing::debug!(
                    topic = topic.name(),
                    response_source = "memory",
                    coalesced_event_count = records.len(),
                    live_path_db_read_count = 0_u64,
                    baseline_age_ms = cached
                        .prompt_cache_baseline_at
                        .map(|started| started.elapsed().as_millis() as u64)
                        .unwrap_or_default(),
                    "prompt cache topic materialized from active projection"
                );
                Some(SubscriptionDispatchEvent { frame })
            }
        };
        if let Some(topic) = bounded_hydration_topic {
            Self::spawn_dashboard_working_conversation_key_hydration(state, topic);
        }
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        Ok(())
    }

    async fn prompt_cache_reconcile_required(&self, topic_key: &str) -> bool {
        self.state
            .lock()
            .await
            .topics
            .get(topic_key)
            .is_some_and(|cached| {
                cached.prompt_cache_reconcile_required
                    || cached.dirty
                    || !cached.prompt_cache_applied_terminal_ids.is_empty()
            })
    }

    async fn expire_dashboard_working_conversations_projection(
        &self,
        state: Arc<AppState>,
        topic_key: &str,
    ) -> Result<(), ApiError> {
        let mut bounded_hydration_topic = None;
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(topic_key) else {
                return Ok(());
            };
            if cached.dirty {
                return Ok(());
            }
            let Some(DashboardTopicMaterializer::WorkingConversations { state }) =
                cached.dashboard_materializer.as_ref()
            else {
                return Ok(());
            };
            let state = state.clone();
            let changed = state
                .lock()
                .expect("working conversations materializer state lock")
                .expire(Utc::now());
            if !changed {
                return Ok(());
            }
            // Expiring an item can uncover a persisted candidate outside this page. Refill only
            // the bounded active page rather than rebuilding the entire working selection.
            cached.prompt_cache_candidate_refill_required = true;
            if !cached.prompt_cache_key_hydration_scheduled {
                cached.prompt_cache_key_hydration_scheduled = true;
                bounded_hydration_topic = Some(cached.topic.clone());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            self.dashboard_topology_counters
                .record_materialization(cached.topic.name(), false);
            let frame = Arc::new(
                self.serialize_frame(
                    cached.descriptor.clone(),
                    topic_key.to_string(),
                    cached.schema_epoch.clone(),
                    next_cursor,
                    state
                        .lock()
                        .expect("working conversations materializer state lock")
                        .serialize()?,
                )?,
            );
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            Some(SubscriptionDispatchEvent { frame })
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        if let Some(topic) = bounded_hydration_topic {
            Self::spawn_dashboard_working_conversation_key_hydration(state, topic);
        }
        Ok(())
    }

    async fn expire_prompt_cache_topic_window(&self, topic_key: &str) -> Result<(), ApiError> {
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(topic_key) else {
                return Ok(());
            };
            if cached.dirty {
                // A cursor gap makes the retained frame incomplete. Keep it byte-for-byte as
                // last-good until the bounded reconcile below replaces it.
                return Ok(());
            }
            let now = Utc::now();
            let activity_cutoff = match cached.topic {
                SubscriptionTopic::PromptCacheWindow {
                    selection: PromptCacheConversationSelection::Count(_),
                    ..
                }
                | SubscriptionTopic::PromptCacheStickyWindow {
                    selection: AccountStickyKeySelection::Count(_),
                    ..
                } => Some(now - ChronoDuration::hours(24)),
                SubscriptionTopic::PromptCacheWindow {
                    selection: PromptCacheConversationSelection::ActivityWindowHours(hours),
                    ..
                } => Some(now - ChronoDuration::hours(hours)),
                SubscriptionTopic::PromptCacheWindow {
                    selection: PromptCacheConversationSelection::ActivityWindowMinutes(minutes),
                    ..
                } => Some(now - ChronoDuration::minutes(minutes)),
                SubscriptionTopic::PromptCacheStickyWindow {
                    selection: AccountStickyKeySelection::ActivityWindow(hours),
                    ..
                } => Some(now - ChronoDuration::hours(hours)),
                _ => None,
            };
            let Some(conversations) = cached
                .snapshot_payload
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            else {
                return Ok(());
            };
            let before = conversations.len();
            if let Some(cutoff) = activity_cutoff {
                conversations.retain(|conversation| {
                    conversation
                        .get("lastActivityAt")
                        .and_then(Value::as_str)
                        .and_then(parse_to_utc_datetime)
                        .is_some_and(|last_activity| last_activity >= cutoff)
                });
            }
            let mut changed = conversations.len() != before;
            if changed {
                cached.prompt_cache_reconcile_required = true;
            }
            let request_cutoff = now - ChronoDuration::hours(24);
            for conversation in conversations {
                let Some(points) = conversation
                    .get_mut("last24hRequests")
                    .and_then(Value::as_array_mut)
                else {
                    continue;
                };
                let before = points.len();
                points.retain(|point| {
                    point
                        .get("occurredAt")
                        .and_then(Value::as_str)
                        .and_then(parse_to_utc_datetime)
                        .is_some_and(|occurred_at| occurred_at >= request_cutoff)
                });
                changed |= points.len() != before;
                let mut cumulative = 0_i64;
                for point in points {
                    cumulative = cumulative.saturating_add(
                        point
                            .get("requestTokens")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    );
                    if let Some(point) = point.as_object_mut() {
                        point.insert("cumulativeTokens".to_string(), Value::from(cumulative));
                    }
                }
            }
            if !changed {
                return Ok(());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.to_string(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&cached.snapshot_payload)?,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: now,
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            Some(SubscriptionDispatchEvent { frame })
        };
        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch);
        }
        Ok(())
    }

    async fn apply_prompt_cache_binding_projection(
        &self,
        state: Arc<AppState>,
        prompt_cache_key: &str,
    ) -> Result<(), ApiError> {
        let active_topic_keys = {
            let guard = self.state.lock().await;
            let mut topic_keys = Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::PromptCacheWindow,
            );
            topic_keys.extend(Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::DashboardWorkingConversationsProjection,
            ));
            topic_keys
                .into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };
        if active_topic_keys.is_empty() {
            return Ok(());
        }
        let binding = load_prompt_cache_conversation_binding_response_for_key(
            state.as_ref(),
            prompt_cache_key.to_string(),
        )
        .await?;
        let mut dispatches = Vec::new();
        let mut reconciles = Vec::new();
        let mut key_hydrations = Vec::new();
        let mut binding_payload = None;
        let mut guard = self.state.lock().await;
        for topic_key in active_topic_keys {
            if guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                continue;
            }
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                continue;
            };
            if cached.dirty {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            }
            let working_state = match cached.dashboard_materializer.as_ref() {
                Some(DashboardTopicMaterializer::WorkingConversations { state }) => {
                    Some(state.clone())
                }
                _ => None,
            };
            let is_working_conversations = working_state.is_some();
            if !is_working_conversations {
                cached.prompt_cache_bounded_key_hydration_count = cached
                    .prompt_cache_bounded_key_hydration_count
                    .saturating_add(1);
            }
            let changed = if let Some(working_state) = working_state.as_ref() {
                let Some(changed) = working_state
                    .lock()
                    .expect("working conversations materializer state lock")
                    .apply_binding(prompt_cache_key, &binding)
                else {
                    cached
                        .prompt_cache_pending_key_hydrations
                        .insert(prompt_cache_key.to_string());
                    cached.prompt_cache_candidate_refill_required = true;
                    if !cached.prompt_cache_key_hydration_scheduled {
                        cached.prompt_cache_key_hydration_scheduled = true;
                        key_hydrations.push(cached.topic.clone());
                    }
                    continue;
                };
                changed
            } else {
                if binding_payload.is_none() {
                    binding_payload = Some(serde_json::to_value(&binding)?);
                }
                let Some(changed) = patch_prompt_cache_binding_payload(
                    &mut cached.snapshot_payload,
                    prompt_cache_key,
                    binding_payload
                        .as_ref()
                        .expect("serialized binding payload"),
                ) else {
                    cached.prompt_cache_reconcile_required = true;
                    if !cached.prompt_cache_reconcile_scheduled {
                        cached.prompt_cache_reconcile_scheduled = true;
                        reconciles.push(cached.topic.clone());
                    }
                    continue;
                };
                changed
            };
            if !changed {
                continue;
            }
            let next_cursor = cached.cursor.saturating_add(1);
            if is_working_conversations {
                self.dashboard_topology_counters
                    .record_materialization(cached.topic.name(), false);
            }
            let serialized_payload = if let Some(working_state) = working_state {
                working_state
                    .lock()
                    .expect("working conversations materializer state lock")
                    .serialize()?
            } else {
                serde_json::to_vec(&cached.snapshot_payload)?
            };
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized_payload,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            dispatches.push(SubscriptionDispatchEvent { frame });
        }
        drop(guard);
        for dispatch in dispatches {
            let _ = self.broadcaster.send(dispatch);
        }
        for topic in reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
        for topic in key_hydrations {
            Self::spawn_dashboard_working_conversation_key_hydration(state.clone(), topic);
        }
        Ok(())
    }

    async fn apply_prompt_cache_sticky_route_projection(
        &self,
        state: Arc<AppState>,
        sticky_key: &str,
        previous_upstream_account_id: i64,
        upstream_account_id: i64,
    ) -> Result<(), ApiError> {
        let mut dispatches = Vec::new();
        let mut reconciles = Vec::new();
        let mut guard = self.state.lock().await;
        let active_topic_keys = Self::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::PromptCacheStickyWindow,
        );
        if active_topic_keys.is_empty() {
            return Ok(());
        }
        for topic_key in active_topic_keys {
            if guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                continue;
            }
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                continue;
            };
            if cached.dirty {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            }
            let SubscriptionTopic::PromptCacheStickyWindow { account_id, .. } = cached.topic else {
                continue;
            };
            let Some(conversations) = cached
                .snapshot_payload
                .get_mut("conversations")
                .and_then(Value::as_array_mut)
            else {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            };
            let before = conversations.len();
            if account_id == previous_upstream_account_id && account_id != upstream_account_id {
                conversations.retain(|conversation| {
                    conversation.get("stickyKey").and_then(Value::as_str) != Some(sticky_key)
                });
            }
            if account_id == upstream_account_id
                && !conversations.iter().any(|conversation| {
                    conversation.get("stickyKey").and_then(Value::as_str) == Some(sticky_key)
                })
            {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
            }
            if conversations.len() == before {
                continue;
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&cached.snapshot_payload)?,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            cached.prompt_cache_response_source = "memory";
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            dispatches.push(SubscriptionDispatchEvent { frame });
        }
        drop(guard);
        for dispatch in dispatches {
            let _ = self.broadcaster.send(dispatch);
        }
        for topic in reconciles {
            Self::spawn_prompt_cache_topic_reconcile(state.clone(), topic);
        }
        Ok(())
    }

    async fn clear_dashboard_activity_topic_refresh_flag(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.refresh_scheduled = false;
        }
    }

    async fn mark_dashboard_activity_topic_dirty(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.dirty = true;
            cached.refresh_scheduled = false;
            cached.latest_live_snapshot = None;
        }
    }

    pub(crate) async fn reconcile_dashboard_terminal_window_bases(&self, state: Arc<AppState>) {
        let topics = {
            let mut guard = self.state.lock().await;
            mark_dashboard_terminal_window_rebase_topics(&mut guard);
            let active_subscribers = guard.active_subscribers.clone();
            guard
                .topics
                .iter()
                .filter(|(topic_key, cached)| {
                    cached.dirty
                        && cached.refresh_scheduled
                        && active_subscribers
                            .get(*topic_key)
                            .copied()
                            .unwrap_or_default()
                            > 0
                        && cached.dashboard_materializer.as_ref().is_some_and(
                            DashboardTopicMaterializer::requires_terminal_window_rebase,
                        )
                })
                .map(|(_, cached)| cached.topic.clone())
                .collect::<Vec<_>>()
        };

        for topic in topics {
            tracing::debug!(
                topic = %topic.name(),
                refresh_reason = "runtime_reconcile_window_rebase",
                "rebuilding typed Dashboard base after moving-window boundary"
            );
            if let Err(err) = self
                .refresh_topic_if_active(state.clone(), topic.clone(), true)
                .await
            {
                warn!(
                    ?err,
                    topic = %topic.name(),
                    "runtime Dashboard window rebase failed; retaining last-good frame"
                );
            }
        }
    }

    async fn schedule_dashboard_activity_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let selection = dashboard_activity_snapshot_selection_for_topic(state.as_ref(), &topic)
            .await?
            .expect("dashboard activity refresh selection should exist for open-range topics");
        let topic_key = topic.cache_key()?;
        let delay = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let age = cached.snapshot_built_at.elapsed();
            if age >= DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL {
                cached.refresh_scheduled = false;
                None
            } else if cached.refresh_scheduled {
                return Ok(());
            } else {
                cached.refresh_scheduled = true;
                Some(DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL.saturating_sub(age))
            }
        };

        if let Some(delay) = delay {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                if !hub.has_active_topic_key(&topic_key).await {
                    tracing::debug!(
                        topic = %topic.name(),
                        refresh_outcome = "marked_dirty",
                        "skipping deferred dashboard activity refresh without owner subscribers"
                    );
                    hub.mark_dashboard_activity_topic_dirty(&topic).await;
                    return;
                }
                tracing::debug!(
                    refresh_reason = "scheduled_terminal_refresh",
                    response_source = "memory",
                    selection_fingerprint = dashboard_activity_selection_fingerprint(&selection),
                    "publishing dashboard activity read model after terminal coalescing"
                );
                match hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    Ok(Some(_)) | Ok(None) => {}
                    Err(err) => {
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            "failed to run deferred dashboard activity topic refresh"
                        );
                        hub.clear_dashboard_activity_topic_refresh_flag(&topic)
                            .await;
                    }
                }
            });
            return Ok(());
        }

        tracing::debug!(
            refresh_reason = "scheduled_terminal_refresh",
            response_source = "memory",
            selection_fingerprint = dashboard_activity_selection_fingerprint(&selection),
            "publishing dashboard activity read model after terminal coalescing"
        );
        let _ = self.refresh_topic_if_active(state, topic, true).await?;
        Ok(())
    }

    async fn schedule_summary_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        event_count: u64,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let delay = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 {
                cached.dirty = true;
                cached.latest_live_snapshot = None;
                return Ok(());
            }
            cached.summary_pending_event_count = cached
                .summary_pending_event_count
                .saturating_add(event_count.max(1));
            if cached.summary_refresh_scheduled || cached.summary_refresh_in_flight {
                return Ok(());
            }
            cached.summary_refresh_scheduled = true;
            Duration::from_millis(
                cached
                    .summary_retry_backoff_ms
                    .max(SUMMARY_TOPIC_REFRESH_DEBOUNCE.as_millis() as u64),
            )
        };

        self.spawn_summary_topic_refresh(state, topic, delay);
        Ok(())
    }

    fn spawn_summary_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        delay: Duration,
    ) {
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            hub.run_summary_topic_refresh(state, topic).await;
        });
    }

    async fn run_summary_topic_refresh(&self, state: Arc<AppState>, topic: SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let (event_count, active) = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            if active == 0 {
                cached.dirty = true;
                cached.latest_live_snapshot = None;
                cached.summary_refresh_scheduled = false;
                cached.summary_pending_event_count = 0;
                return;
            }
            cached.summary_refresh_in_flight = true;
            let event_count = std::mem::take(&mut cached.summary_pending_event_count);
            (event_count, active)
        };

        let started = Instant::now();
        let result = match state.sqlite_batch_writer.flush_now(&state.pool).await {
            Ok(()) => {
                self.refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
            }
            Err(err) => Err(ApiError::from(anyhow!(
                "summary topic prerequisite flush failed: {err}"
            ))),
        };
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let mut retry_delay = None;
        let mut coalesced_event_count = event_count;
        let mut refresh_outcome = "published";
        {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            cached.summary_refresh_in_flight = false;
            match result {
                Ok(Some(_)) => {
                    cached.summary_retry_backoff_ms = 0;
                    if cached.summary_pending_event_count > 0 {
                        cached.summary_refresh_scheduled = true;
                        retry_delay = Some(SUMMARY_TOPIC_REFRESH_DEBOUNCE);
                    } else {
                        cached.summary_refresh_scheduled = false;
                    }
                }
                Ok(None) => {
                    cached.dirty = true;
                    cached.latest_live_snapshot = None;
                    cached.summary_refresh_scheduled = false;
                    cached.summary_pending_event_count = 0;
                    refresh_outcome = "marked_dirty";
                }
                Err(err) => {
                    refresh_outcome = "retained_last_good";
                    let backoff_ms = match cached.summary_retry_backoff_ms {
                        0 => 500,
                        500 => 1_000,
                        1_000 => 2_000,
                        _ => 5_000,
                    };
                    cached.summary_retry_backoff_ms = backoff_ms;
                    cached.summary_refresh_scheduled = true;
                    retry_delay = Some(Duration::from_millis(backoff_ms));
                    warn!(
                        ?err,
                        topic = %topic.name(),
                        refresh_outcome,
                        retry_backoff_ms = backoff_ms,
                        last_good_age_ms = cached.snapshot_built_at.elapsed().as_millis() as u64,
                        "summary topic refresh failed; retaining last-good snapshot"
                    );
                }
            }
            coalesced_event_count =
                coalesced_event_count.saturating_add(cached.summary_pending_event_count);
        }
        tracing::debug!(
            topic = %topic.name(),
            active_subscriber_count = active,
            coalesced_event_count,
            build_source = "summary_exact",
            elapsed_ms,
            refresh_outcome,
            "summary topic refresh completed"
        );
        if let Some(delay) = retry_delay {
            self.spawn_summary_topic_refresh(state, topic, delay);
        }
    }

    async fn apply_summary_live_overlay(
        &self,
        topic: &SubscriptionTopic,
        live: DashboardActivityLiveSnapshot,
    ) -> Result<(), ApiError> {
        let SubscriptionTopic::SummaryCurrent {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(());
        };
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let account = upstream_account_id.and_then(|account_id| {
            live.accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(account_id))
        });
        let (count, retry_count, phase_counts, wait_ms) = match account {
            Some(account) => (
                account.in_progress_invocation_count,
                account.retry_invocation_count,
                account.in_progress_phase_counts,
                (account.in_progress_wait_sample_count > 0).then_some(
                    account.in_progress_wait_sum_ms / account.in_progress_wait_sample_count as f64,
                ),
            ),
            None if upstream_account_id.is_some() => {
                (0, 0, InvocationPhaseCountsResponse::default(), None)
            }
            None => (
                live.in_progress_invocation_count,
                live.retry_invocation_count,
                live.in_progress_phase_counts,
                (live.in_progress_wait_sample_count > 0).then_some(
                    live.in_progress_wait_sum_ms / live.in_progress_wait_sample_count as f64,
                ),
            ),
        };

        let topic_key = topic.cache_key()?;
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            cached.latest_live_snapshot = Some(live.clone());
            let Some(object) = cached.snapshot_payload.as_object_mut() else {
                return Ok(());
            };
            let next_values = [
                ("inProgressConversationCount", Value::from(count)),
                ("inProgressRetryConversationCount", Value::from(retry_count)),
                (
                    "inProgressAvgWaitMs",
                    wait_ms.map(Value::from).unwrap_or(Value::Null),
                ),
                ("inProgressPhaseCounts", serde_json::to_value(phase_counts)?),
            ];
            if next_values
                .iter()
                .all(|(key, value)| object.get(*key) == Some(value))
            {
                return Ok(());
            }
            for (key, value) in next_values {
                set_json_field(object, key, value);
            }
            let payload = cached.snapshot_payload.clone();
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&payload)?,
            )?);
            let payload_bytes = frame.payload_bytes.len();
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent { frame }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }

    async fn apply_dashboard_activity_live_overlay(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
        live: DashboardActivityLiveSnapshot,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let mut payload = cached.snapshot_payload.clone();
            if !apply_dashboard_activity_live_overlay_to_payload(
                state.as_ref(),
                &mut payload,
                &live,
            )? {
                return Ok(());
            }

            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&payload)?,
            )?);
            let payload_bytes = frame.payload_bytes.len();
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_payload = payload.clone();
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);

            SubscriptionDispatchEvent { frame }
        };

        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }

    async fn apply_dashboard_network_slice_to_timeseries(
        &self,
        topic: &SubscriptionTopic,
        slice: &DashboardNetworkProjectionSlice,
    ) -> Result<(), ApiError> {
        let SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(());
        };
        let bucket = match upstream_account_id {
            None => slice.network_live_bucket.clone(),
            Some(upstream_account_id) => slice
                .accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(*upstream_account_id))
                .and_then(|account| account.network_live_bucket.clone()),
        };
        let Some(bucket) = bucket else {
            return Ok(());
        };
        let topic_key = topic.cache_key()?;
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let mut payload = cached.snapshot_payload.clone();
            let Some(object) = payload.as_object_mut() else {
                return Ok(());
            };
            let bucket_value = serde_json::to_value(&bucket)?;
            let bucket_start = bucket_value.get("bucketStart").cloned();
            let Some(points) = object.get_mut("points").and_then(Value::as_array_mut) else {
                return Ok(());
            };
            let point_index = points
                .iter()
                .position(|point| point.get("bucketStart") == bucket_start.as_ref())
                .or_else(|| {
                    points.iter().position(|point| {
                        point
                            .get("isLiveBucket")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    })
                });
            let Some(point_index) = point_index else {
                return Ok(());
            };
            let now = Utc::now();
            points[point_index] = bucket_value;
            object.insert(
                "rangeEnd".to_string(),
                Value::String(format_utc_iso_precise(now)),
            );
            object.insert(
                "snapshotId".to_string(),
                Value::from(now.timestamp_millis()),
            );
            let serialized = serde_json::to_vec(&payload)?;
            if cached.snapshot_frame.payload_bytes.as_ref() == serialized.as_slice() {
                return Ok(());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_payload = payload;
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.snapshot_frame = frame.clone();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: now,
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent { frame }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }
}

fn apply_topic_live_overlay_to_payload(
    state: &AppState,
    topic: &SubscriptionTopic,
    payload: &mut Value,
    live: &DashboardActivityLiveSnapshot,
) -> Result<bool, ApiError> {
    if topic.uses_summary_live_overlay() {
        let SubscriptionTopic::SummaryCurrent {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(false);
        };
        let account = upstream_account_id.and_then(|account_id| {
            live.accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(account_id))
        });
        let (count, retry_count, phase_counts, wait_ms) = match account {
            Some(account) => (
                account.in_progress_invocation_count,
                account.retry_invocation_count,
                account.in_progress_phase_counts,
                (account.in_progress_wait_sample_count > 0).then_some(
                    account.in_progress_wait_sum_ms / account.in_progress_wait_sample_count as f64,
                ),
            ),
            None if upstream_account_id.is_some() => {
                (0, 0, InvocationPhaseCountsResponse::default(), None)
            }
            None => (
                live.in_progress_invocation_count,
                live.retry_invocation_count,
                live.in_progress_phase_counts,
                (live.in_progress_wait_sample_count > 0).then_some(
                    live.in_progress_wait_sum_ms / live.in_progress_wait_sample_count as f64,
                ),
            ),
        };
        let Some(object) = payload.as_object_mut() else {
            return Ok(false);
        };
        let next_values = [
            ("inProgressConversationCount", Value::from(count)),
            ("inProgressRetryConversationCount", Value::from(retry_count)),
            (
                "inProgressAvgWaitMs",
                wait_ms.map(Value::from).unwrap_or(Value::Null),
            ),
            ("inProgressPhaseCounts", serde_json::to_value(phase_counts)?),
        ];
        let changed = next_values
            .iter()
            .any(|(key, value)| object.get(*key) != Some(value));
        for (key, value) in next_values {
            set_json_field(object, key, value);
        }
        return Ok(changed);
    }

    if topic.uses_dashboard_activity_live_overlay() {
        return apply_dashboard_activity_live_overlay_to_payload(state, payload, live);
    }

    Ok(false)
}

fn set_json_field(object: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    object.insert(key.to_string(), value);
}

async fn load_persisted_invocation_identities(
    conn: &mut sqlx::SqliteConnection,
    identities: &HashSet<String>,
    baseline_row_id: i64,
) -> Result<HashSet<String>, ApiError> {
    let selectors = identities
        .iter()
        .filter_map(|identity| identity.split_once('\0'))
        .collect::<Vec<_>>();
    let mut persisted = HashSet::new();
    for chunk in selectors.chunks(300) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT invoke_id, occurred_at FROM codex_invocations WHERE id <= ",
        );
        query.push_bind(baseline_row_id);
        query.push(" AND (");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(*invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(*occurred_at)
                .push(")");
        }
        query.push(")");
        for (invoke_id, occurred_at) in query
            .build_query_as::<(String, String)>()
            .fetch_all(&mut *conn)
            .await?
        {
            persisted.insert(format!("{invoke_id}\0{occurred_at}"));
        }
    }
    Ok(persisted)
}

fn patch_prompt_cache_binding_payload(
    payload: &mut Value,
    prompt_cache_key: &str,
    binding: &Value,
) -> Option<bool> {
    let conversation = payload
        .get_mut("conversations")?
        .as_array_mut()?
        .iter_mut()
        .find(|conversation| {
            conversation.get("promptCacheKey").and_then(Value::as_str) == Some(prompt_cache_key)
        })?
        .as_object_mut()?;
    let binding_kind = binding.get("bindingKind").and_then(Value::as_str);
    let manual_binding = binding_kind
        .filter(|kind| *kind != "none")
        .map(|kind| {
            serde_json::json!({
                "bindingKind": kind,
                "groupName": binding.get("groupName").cloned().unwrap_or(Value::Null),
                "upstreamAccountId": binding.get("upstreamAccountId").cloned().unwrap_or(Value::Null),
                "upstreamAccountName": binding.get("upstreamAccountName").cloned().unwrap_or(Value::Null),
            })
        })
        .unwrap_or(Value::Null);
    let replacements = [
        (
            "hasEncryptedSessionOwner",
            binding
                .get("hasEncryptedSessionOwner")
                .cloned()
                .unwrap_or(Value::Bool(false)),
        ),
        (
            "encryptedOwnerAccountId",
            binding
                .get("encryptedOwnerAccountId")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "encryptedOwnerAccountName",
            binding
                .get("encryptedOwnerAccountName")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "encryptedOwnerGroupName",
            binding
                .get("encryptedOwnerGroupName")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("manualBinding", manual_binding),
    ];
    let changed = replacements
        .iter()
        .any(|(field, value)| conversation.get(*field) != Some(value));
    for (field, value) in replacements {
        if value.is_null() {
            conversation.remove(field);
        } else {
            conversation.insert(field.to_string(), value);
        }
    }
    Some(changed)
}

fn apply_prompt_cache_records_to_payload(
    topic: &SubscriptionTopic,
    payload: &mut Value,
    records: &[PromptCacheTopicDelta],
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) -> Result<bool, ApiError> {
    let Some(conversations) = payload
        .as_object_mut()
        .and_then(|object| object.get_mut("conversations"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(false);
    };
    let mut changed = false;
    for record in records {
        let key = match topic {
            SubscriptionTopic::PromptCacheWindow { .. } => record.prompt_cache_key.as_deref(),
            SubscriptionTopic::PromptCacheStickyWindow { .. } => record.sticky_key.as_deref(),
            _ => None,
        };
        let Some(key) = key else {
            continue;
        };
        if let SubscriptionTopic::PromptCacheStickyWindow { account_id, .. } = topic
            && record.upstream_account_id != Some(*account_id)
        {
            continue;
        }
        let key_field = match topic {
            SubscriptionTopic::PromptCacheWindow { .. } => "promptCacheKey",
            SubscriptionTopic::PromptCacheStickyWindow { .. } => "stickyKey",
            _ => return Ok(false),
        };
        let occurred_at = Value::String(record.occurred_at.clone());
        let status = record.status.as_str();
        let is_terminal = record.is_terminal;
        let is_success = record.is_success;
        let request_tokens = record.request_tokens;
        let conversation_index = conversations.iter().position(|conversation| {
            conversation.get(key_field).and_then(Value::as_str) == Some(key)
        });
        if record.is_runtime_removed {
            let Some(index) = conversation_index else {
                continue;
            };
            let Some(conversation) = conversations[index].as_object_mut() else {
                continue;
            };
            let Some(recent) = conversation
                .get_mut("recentInvocations")
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            let count_before = recent.len();
            recent.retain(|item| {
                item.get("invokeId").and_then(Value::as_str) != Some(record.invoke_id.as_str())
                    || item.get("occurredAt") != Some(&occurred_at)
            });
            changed |= recent.len() != count_before;
            continue;
        }
        let Some(preview) = record.preview.as_ref() else {
            continue;
        };
        let preview = serde_json::to_value(preview)?;
        let index = match conversation_index {
            Some(index) => index,
            None => {
                let mut conversation = serde_json::Map::new();
                conversation.insert(key_field.to_string(), Value::String(key.to_string()));
                conversation.insert("requestCount".to_string(), Value::from(0));
                conversation.insert("totalTokens".to_string(), Value::from(0));
                conversation.insert("totalCost".to_string(), Value::from(0.0));
                conversation.insert("createdAt".to_string(), occurred_at.clone());
                conversation.insert("lastActivityAt".to_string(), occurred_at.clone());
                conversation.insert("recentInvocations".to_string(), Value::Array(Vec::new()));
                conversation.insert("last24hRequests".to_string(), Value::Array(Vec::new()));
                if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. }) {
                    conversation.insert("hasEncryptedSessionOwner".to_string(), Value::Bool(false));
                    conversation.insert("upstreamAccounts".to_string(), Value::Array(Vec::new()));
                }
                conversations.push(Value::Object(conversation));
                conversations.len() - 1
            }
        };
        let Some(conversation) = conversations[index].as_object_mut() else {
            continue;
        };
        let recent = conversation
            .entry("recentInvocations")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("prompt cache recentInvocations is an array");
        let existing_index = recent.iter().position(|item| {
            item.get("invokeId").and_then(Value::as_str)
                == preview.get("invokeId").and_then(Value::as_str)
                && item.get("occurredAt") == Some(&occurred_at)
        });
        let already_terminal = applied_terminal_ids.contains(&record.identity)
            || (record.row_id > 0 && record.row_id <= baseline_row_id);
        if let Some(index) = existing_index {
            recent[index] = preview.clone();
        } else {
            recent.push(preview.clone());
        }
        recent.sort_by(|left, right| {
            right
                .get("occurredAt")
                .and_then(Value::as_str)
                .cmp(&left.get("occurredAt").and_then(Value::as_str))
        });
        let recent_limit = match topic {
            SubscriptionTopic::PromptCacheWindow {
                recent_invocation_limit,
                ..
            } => recent_invocation_limit.unwrap_or(16).max(0) as usize,
            SubscriptionTopic::PromptCacheStickyWindow { .. } => 5,
            _ => 0,
        };
        recent.truncate(recent_limit);

        if conversation
            .get("lastActivityAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
        {
            conversation.insert("lastActivityAt".to_string(), occurred_at.clone());
        }
        if conversation
            .get("createdAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next < current))
        {
            conversation.insert("createdAt".to_string(), occurred_at.clone());
        }
        if is_terminal && !already_terminal {
            applied_terminal_ids.insert(record.identity.clone());
            increment_json_i64(conversation, "requestCount", 1);
            increment_json_i64(conversation, "totalTokens", request_tokens);
            increment_json_f64(conversation, "totalCost", record.cost);
            if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. }) {
                if conversation
                    .get("lastTerminalAt")
                    .and_then(Value::as_str)
                    .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
                {
                    conversation.insert("lastTerminalAt".to_string(), occurred_at.clone());
                }
                apply_prompt_cache_account_delta(conversation, record, &occurred_at);
            }
            let points = conversation
                .entry("last24hRequests")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("prompt cache last24hRequests is an array");
            if !points.iter().any(|point| {
                point.get("occurredAt") == Some(&occurred_at)
                    && point.get("requestTokens").and_then(Value::as_i64) == Some(request_tokens)
            }) {
                let mut point = serde_json::json!({
                    "occurredAt": occurred_at,
                    "status": status,
                    "isSuccess": is_success,
                    "requestTokens": request_tokens,
                    "cumulativeTokens": 0,
                });
                if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. })
                    && let Some(point) = point.as_object_mut()
                {
                    point.insert(
                        "outcome".to_string(),
                        Value::String(if is_success { "success" } else { "failure" }.to_string()),
                    );
                }
                points.push(point);
                points.sort_by(|left, right| {
                    left.get("occurredAt")
                        .and_then(Value::as_str)
                        .cmp(&right.get("occurredAt").and_then(Value::as_str))
                });
                let mut cumulative = 0_i64;
                for point in points {
                    cumulative = cumulative.saturating_add(
                        point
                            .get("requestTokens")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    );
                    if let Some(point) = point.as_object_mut() {
                        point.insert("cumulativeTokens".to_string(), Value::from(cumulative));
                    }
                }
            }
        }
        changed = true;
    }

    conversations.sort_by(|left, right| {
        right
            .get("lastActivityAt")
            .and_then(Value::as_str)
            .cmp(&left.get("lastActivityAt").and_then(Value::as_str))
    });
    let display_limit = match topic {
        SubscriptionTopic::PromptCacheWindow { selection, .. } => selection.display_limit(),
        SubscriptionTopic::PromptCacheStickyWindow { selection, .. } => selection.display_limit(),
        _ => 0,
    };
    conversations.truncate(display_limit.max(0) as usize);
    Ok(changed)
}

fn increment_json_i64(object: &mut serde_json::Map<String, Value>, field: &str, delta: i64) {
    let current = object
        .get(field)
        .and_then(Value::as_i64)
        .unwrap_or_default();
    object.insert(
        field.to_string(),
        Value::from(current.saturating_add(delta)),
    );
}

fn prompt_cache_delta_needs_replay(
    delta: &PromptCacheTopicDelta,
    persisted_identities: &HashSet<String>,
) -> bool {
    !persisted_identities.contains(&delta.identity)
}

fn increment_json_f64(object: &mut serde_json::Map<String, Value>, field: &str, delta: f64) {
    let current = object
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_default();
    object.insert(field.to_string(), Value::from(current + delta));
}

fn apply_prompt_cache_account_delta(
    conversation: &mut serde_json::Map<String, Value>,
    record: &PromptCacheTopicDelta,
    occurred_at: &Value,
) {
    let accounts = conversation
        .entry("upstreamAccounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("prompt cache upstreamAccounts is an array");
    let index = accounts.iter().position(|account| {
        account.get("upstreamAccountId").and_then(Value::as_i64) == record.upstream_account_id
    });
    let index = index.unwrap_or_else(|| {
        accounts.push(serde_json::json!({
            "upstreamAccountId": record.upstream_account_id,
            "upstreamAccountName": record.upstream_account_name,
            "requestCount": 0,
            "totalTokens": 0,
            "totalCost": 0.0,
            "lastActivityAt": occurred_at,
        }));
        accounts.len() - 1
    });
    if let Some(account) = accounts[index].as_object_mut() {
        increment_json_i64(account, "requestCount", 1);
        increment_json_i64(account, "totalTokens", record.request_tokens);
        increment_json_f64(account, "totalCost", record.cost);
        if account
            .get("lastActivityAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
        {
            account.insert("lastActivityAt".to_string(), occurred_at.clone());
        }
    }
}

fn set_json_optional_field(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<Value>,
) {
    match value {
        Some(value) => {
            object.insert(key.to_string(), value);
        }
        None => {
            object.remove(key);
        }
    }
}

fn dashboard_activity_payload_exact_range(payload: &Value) -> Option<ExactUtcRange> {
    let object = payload.as_object()?;
    let range_start = object.get("rangeStart")?.as_str()?;
    let range_end = object.get("rangeEnd")?.as_str()?;
    Some(ExactUtcRange {
        start: parse_to_utc_datetime(range_start)?,
        end: parse_to_utc_datetime(range_end)?,
    })
}

async fn dashboard_activity_snapshot_selection_for_topic(
    state: &AppState,
    topic: &SubscriptionTopic,
) -> Result<Option<DashboardActivitySnapshotSelection>, ApiError> {
    let SubscriptionTopic::DashboardActivityCurrent {
        range,
        time_zone,
        recent_limit,
        include_accounts,
        include_recent,
    } = topic
    else {
        return Ok(None);
    };

    if range == "yesterday" {
        return Ok(None);
    }

    let recent_limit = validate_dashboard_activity_params(
        "dashboard.activity.current",
        range,
        Some(*recent_limit),
    )?;
    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
    let exact_range = resolve_dashboard_activity_cached_range(range, reporting_tz)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    Ok(Some(build_dashboard_activity_snapshot_selection(
        range,
        exact_range,
        reporting_tz,
        source_scope,
        recent_limit,
        *include_accounts,
        *include_recent,
    )))
}

fn dashboard_activity_account_sort_tuple(value: &Value) -> (i64, Option<&str>, i64) {
    let total_tokens = value
        .get("totalTokens")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let first_recent_occurred_at = value
        .get("recentInvocations")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("occurredAt"))
        .and_then(Value::as_str);
    let upstream_account_id = value
        .get("upstreamAccountId")
        .and_then(Value::as_i64)
        .unwrap_or(i64::MIN);
    (total_tokens, first_recent_occurred_at, upstream_account_id)
}

fn apply_dashboard_activity_live_overlay_to_payload(
    state: &AppState,
    payload: &mut Value,
    live: &DashboardActivityLiveSnapshot,
) -> Result<bool, ApiError> {
    let request_range = dashboard_activity_payload_exact_range(payload);
    let Some(root) = payload.as_object_mut() else {
        return Ok(false);
    };
    let model_performance_available = root
        .get("summary")
        .and_then(Value::as_object)
        .and_then(|summary| summary.get("modelPerformance"))
        .and_then(Value::as_object)
        .and_then(|model| model.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    root.insert("liveRevision".to_string(), json!(live.revision));
    set_json_optional_field(
        root,
        "networkLiveBucket",
        live.network_live_bucket
            .clone()
            .map(serde_json::to_value)
            .transpose()?,
    );
    set_json_optional_field(
        root,
        "networkRealtimeRate",
        live.network_realtime_rate
            .clone()
            .map(serde_json::to_value)
            .transpose()?,
    );
    let current_snapshot_by_account = state
        .dashboard_network_speed_cache
        .snapshot_dashboard_activity_accounts(Utc::now());
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    {
        let Some(summary) = root.get_mut("summary").and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        let Some(stats) = summary.get_mut("stats").and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        stats.insert(
            "inProgressConversationCount".to_string(),
            json!(live.in_progress_invocation_count),
        );
        stats.insert(
            "inProgressRetryConversationCount".to_string(),
            json!(live.retry_invocation_count),
        );
        stats.insert(
            "inProgressPhaseCounts".to_string(),
            serde_json::to_value(live.in_progress_phase_counts)?,
        );
        summary.insert(
            "tokensPerMinute".to_string(),
            json!(current_snapshot_summary.qualified_tokens.max(0) as f64),
        );
        summary.insert(
            "spendRate".to_string(),
            json!(current_snapshot_summary.total_cost.max(0.0)),
        );
        set_json_optional_field(
            summary,
            "currentFirstResponseByteTotalAvgMs",
            current_snapshot_summary
                .first_response_byte_total_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            summary,
            "currentFirstTokenAvgMs",
            current_snapshot_summary
                .first_token_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            summary,
            "currentAvgTotalMs",
            current_snapshot_summary
                .avg_total_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            summary,
            "currentAvgResponseMs",
            current_snapshot_summary
                .avg_response_duration_ms()
                .map(|value| json!(value)),
        );
    }

    let Some(accounts_value) = root.get_mut("accounts") else {
        return Ok(true);
    };
    let Some(accounts) = accounts_value.as_array_mut() else {
        return Ok(true);
    };

    let live_accounts = live
        .accounts
        .iter()
        .map(|account| (account.account_key.as_str(), account))
        .collect::<HashMap<_, _>>();
    let mut existing_account_keys = HashSet::new();

    for account in accounts.iter_mut() {
        let Some(account_object) = account.as_object_mut() else {
            continue;
        };
        let Some(account_key) = account_object.get("accountKey").and_then(Value::as_str) else {
            continue;
        };
        existing_account_keys.insert(account_key.to_string());
        let live_account = live_accounts.get(account_key).copied();
        let upstream_account_id = account_object
            .get("upstreamAccountId")
            .and_then(Value::as_i64);
        let current_snapshot = current_snapshot_by_account
            .get(&upstream_account_id)
            .copied()
            .unwrap_or_default();

        set_json_field(
            account_object,
            "inProgressInvocationCount",
            json!(live_account.map_or(0, |account| account.in_progress_invocation_count)),
        );
        set_json_field(
            account_object,
            "inProgressPhaseCounts",
            serde_json::to_value(
                live_account
                    .map(|account| account.in_progress_phase_counts)
                    .unwrap_or_default(),
            )?,
        );
        set_json_field(
            account_object,
            "retryInvocationCount",
            json!(live_account.map_or(0, |account| account.retry_invocation_count)),
        );
        set_json_field(
            account_object,
            "uploadBytesPerSecond",
            json!(live_account.map_or(0.0, |account| account.upload_bytes_per_second)),
        );
        set_json_field(
            account_object,
            "downloadBytesPerSecond",
            json!(live_account.map_or(0.0, |account| account.download_bytes_per_second)),
        );
        set_json_field(
            account_object,
            "tokensPerMinute",
            json!(current_snapshot.qualified_tokens.max(0) as f64),
        );
        set_json_field(
            account_object,
            "spendRate",
            json!(current_snapshot.total_cost.max(0.0)),
        );
        set_json_optional_field(
            account_object,
            "currentFirstResponseByteTotalAvgMs",
            current_snapshot
                .first_response_byte_total_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            account_object,
            "currentFirstTokenAvgMs",
            current_snapshot
                .first_token_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            account_object,
            "currentAvgTotalMs",
            current_snapshot.avg_total_ms().map(|value| json!(value)),
        );
        set_json_optional_field(
            account_object,
            "currentAvgResponseMs",
            current_snapshot
                .avg_response_duration_ms()
                .map(|value| json!(value)),
        );
        if let Some(live_account) = live_account {
            let current_request_count = account_object
                .get("requestCount")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            set_json_field(
                account_object,
                "requestCount",
                json!(current_request_count.max(live_account.in_progress_invocation_count.max(0))),
            );
        }
    }

    if let Some(request_range) = request_range {
        for live_account in &live.accounts {
            if existing_account_keys.contains(&live_account.account_key) {
                continue;
            }
            let current_snapshot = current_snapshot_by_account
                .get(&live_account.upstream_account_id)
                .copied()
                .unwrap_or_default();
            let placeholder = dashboard_activity_account_from_live(
                live_account,
                None,
                request_range,
                current_snapshot,
                model_performance_available,
                None,
                Vec::new(),
            );
            accounts.push(serde_json::to_value(placeholder)?);
        }
        accounts.sort_by(|left, right| {
            let left_key = dashboard_activity_account_sort_tuple(left);
            let right_key = dashboard_activity_account_sort_tuple(right);
            right_key
                .0
                .cmp(&left_key.0)
                .then_with(|| right_key.1.cmp(&left_key.1))
                .then_with(|| right_key.2.cmp(&left_key.2))
        });
    }

    Ok(true)
}

fn dashboard_activity_response_exact_range(
    response: &DashboardActivityResponse,
) -> Option<ExactUtcRange> {
    Some(ExactUtcRange {
        start: parse_to_utc_datetime(&response.range_start)?,
        end: parse_to_utc_datetime(&response.range_end)?,
    })
}

fn apply_dashboard_activity_slices(
    response: &mut DashboardActivityResponse,
    current: Option<&DashboardCurrentProjectionSlice>,
    network: Option<&DashboardNetworkProjectionSlice>,
) {
    if let Some(current) = current {
        response.live_revision = current.revision;
        response.summary.stats.in_progress_conversation_count =
            Some(current.in_progress_invocation_count);
        response.summary.stats.in_progress_retry_conversation_count =
            Some(current.retry_invocation_count);
        response.summary.stats.in_progress_avg_wait_ms = (current.in_progress_wait_sample_count
            > 0)
        .then_some(current.in_progress_wait_sum_ms / current.in_progress_wait_sample_count as f64);
        response.summary.stats.in_progress_phase_counts = Some(current.in_progress_phase_counts);

        let exact_range = dashboard_activity_response_exact_range(response);
        let model_performance_available = response.summary.model_performance.available;
        if let Some(accounts) = response.accounts.as_mut() {
            let current_by_key = current
                .accounts
                .iter()
                .map(|account| (account.account_key.as_str(), account))
                .collect::<HashMap<_, _>>();
            let existing_account_keys = accounts
                .iter()
                .map(|account| account.account_key.clone())
                .collect::<HashSet<_>>();

            for account in accounts.iter_mut() {
                apply_dashboard_current_slice_to_activity_account(
                    account,
                    current_by_key.get(account.account_key.as_str()).copied(),
                );
            }

            if let Some(exact_range) = exact_range {
                for account in &current.accounts {
                    if existing_account_keys.contains(&account.account_key) {
                        continue;
                    }
                    let live_account = DashboardActivityLiveAccount {
                        account_key: account.account_key.clone(),
                        upstream_account_id: account.upstream_account_id,
                        upstream_account_name: account.upstream_account_name.clone(),
                        in_progress_invocation_count: account.in_progress_invocation_count,
                        in_progress_phase_counts: account.in_progress_phase_counts,
                        retry_invocation_count: account.retry_invocation_count,
                        in_progress_wait_sum_ms: account.in_progress_wait_sum_ms,
                        in_progress_wait_sample_count: account.in_progress_wait_sample_count,
                        upload_bytes_per_second: 0.0,
                        download_bytes_per_second: 0.0,
                        network_live_bucket: None,
                    };
                    accounts.push(dashboard_activity_account_from_live(
                        &live_account,
                        None,
                        exact_range,
                        network
                            .and_then(|slice| {
                                slice
                                    .current_snapshot_by_account
                                    .get(&account.upstream_account_id)
                                    .copied()
                            })
                            .unwrap_or_default(),
                        model_performance_available,
                        None,
                        Vec::new(),
                    ));
                }
                sort_dashboard_activity_accounts(accounts);
            }
        }
    }

    if let Some(network) = network {
        response.network_live_bucket = network.network_live_bucket.clone();
        response.network_realtime_rate = network.network_realtime_rate.clone();
        response.summary.tokens_per_minute =
            Some(network.current_snapshot.qualified_tokens.max(0) as f64);
        response.summary.spend_rate = Some(network.current_snapshot.total_cost.max(0.0));
        response.summary.current_first_response_byte_total_avg_ms =
            network.current_snapshot.first_response_byte_total_avg_ms();
        response.summary.current_first_token_avg_ms = network.current_snapshot.first_token_avg_ms();
        response.summary.current_avg_total_ms = network.current_snapshot.avg_total_ms();
        response.summary.current_avg_response_ms =
            network.current_snapshot.avg_response_duration_ms();
        if let Some(accounts) = response.accounts.as_mut() {
            let network_by_account = network
                .accounts
                .iter()
                .map(|account| (account.upstream_account_id, account))
                .collect::<HashMap<_, _>>();
            for account in accounts {
                let network_account = network_by_account
                    .get(&account.upstream_account_id)
                    .copied();
                account.upload_bytes_per_second =
                    network_account.map_or(0.0, |value| value.upload_bytes_per_second);
                account.download_bytes_per_second =
                    network_account.map_or(0.0, |value| value.download_bytes_per_second);
                apply_dashboard_current_rate_to_activity_account(
                    account,
                    network
                        .current_snapshot_by_account
                        .get(&account.upstream_account_id)
                        .copied()
                        .unwrap_or_default(),
                );
            }
        }
    }
}

fn apply_dashboard_current_slice_to_activity_account(
    account: &mut DashboardActivityAccountResponse,
    current: Option<&DashboardCurrentProjectionAccountSlice>,
) {
    account.in_progress_invocation_count =
        Some(current.map_or(0, |value| value.in_progress_invocation_count));
    account.in_progress_phase_counts = Some(
        current
            .map(|value| value.in_progress_phase_counts)
            .unwrap_or_default(),
    );
    account.retry_invocation_count = Some(current.map_or(0, |value| value.retry_invocation_count));
    if let Some(current) = current {
        account.request_count = account
            .request_count
            .max(current.in_progress_invocation_count.max(0));
    }
}

fn apply_dashboard_current_rate_to_activity_account(
    account: &mut DashboardActivityAccountResponse,
    current: DashboardActivityCurrentSnapshot,
) {
    account.tokens_per_minute = Some(current.qualified_tokens.max(0) as f64);
    account.spend_rate = Some(current.total_cost.max(0.0));
    account.current_first_response_byte_total_avg_ms = current.first_response_byte_total_avg_ms();
    account.current_first_token_avg_ms = current.first_token_avg_ms();
    account.current_avg_total_ms = current.avg_total_ms();
    account.current_avg_response_ms = current.avg_response_duration_ms();
}

fn apply_dashboard_current_slice_to_summary_response(
    response: &mut StatsResponse,
    upstream_account_id: Option<i64>,
    current: Option<&DashboardCurrentProjectionSlice>,
) {
    let Some(current) = current else {
        return;
    };
    let account = upstream_account_id.and_then(|account_id| {
        current
            .accounts
            .iter()
            .find(|account| account.upstream_account_id == Some(account_id))
    });
    let (count, retry_count, phase_counts, wait_ms) = match account {
        Some(account) => (
            account.in_progress_invocation_count,
            account.retry_invocation_count,
            account.in_progress_phase_counts,
            (account.in_progress_wait_sample_count > 0).then_some(
                account.in_progress_wait_sum_ms / account.in_progress_wait_sample_count as f64,
            ),
        ),
        None if upstream_account_id.is_some() => {
            (0, 0, InvocationPhaseCountsResponse::default(), None)
        }
        None => (
            current.in_progress_invocation_count,
            current.retry_invocation_count,
            current.in_progress_phase_counts,
            (current.in_progress_wait_sample_count > 0).then_some(
                current.in_progress_wait_sum_ms / current.in_progress_wait_sample_count as f64,
            ),
        ),
    };
    response.in_progress_conversation_count = Some(count);
    response.in_progress_retry_conversation_count = Some(retry_count);
    response.in_progress_avg_wait_ms = wait_ms;
    response.in_progress_phase_counts = Some(phase_counts);
}

fn terminal_delta_matches_source_scope(
    delta: &DashboardActivityTerminalDelta,
    source_scope: InvocationSourceScope,
) -> bool {
    source_scope != InvocationSourceScope::ProxyOnly || delta.source == SOURCE_PROXY
}

fn terminal_delta_is_within_range(
    delta: &DashboardActivityTerminalDelta,
    range: ExactUtcRange,
) -> bool {
    parse_to_utc_datetime(&delta.occurred_at)
        .is_some_and(|occurred_at| occurred_at >= range.start && occurred_at < range.end)
}

fn apply_dashboard_terminal_slice_to_summary_response(
    response: &mut StatsResponse,
    terminal_sequence: &mut u64,
    window: &SummaryWindow,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    slice: &DashboardTerminalProjectionSlice,
) {
    let range = summary_window_range(window, reporting_tz, Utc::now())
        .ok()
        .flatten()
        .map(|(start, end)| ExactUtcRange { start, end });
    for delta in &slice.deltas {
        if !terminal_delta_matches_source_scope(delta, source_scope)
            || delta.terminal_sequence <= *terminal_sequence
            || upstream_account_id
                .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
            || range.is_some_and(|range| !terminal_delta_is_within_range(delta, range))
        {
            continue;
        }
        apply_dashboard_activity_terminal_delta_to_stats(response, delta);
        *terminal_sequence = (*terminal_sequence).max(delta.terminal_sequence);
    }
}

fn dashboard_network_timeseries_live_point<'a>(
    base: &'a DashboardNetworkTimeseriesResponse,
    upstream_account_id: Option<i64>,
    network: Option<&'a DashboardNetworkProjectionSlice>,
) -> Option<(usize, &'a DashboardNetworkTimeseriesPointResponse)> {
    let network = network?;
    let bucket = match upstream_account_id {
        None => network.network_live_bucket.as_ref(),
        Some(upstream_account_id) => network
            .accounts
            .iter()
            .find(|account| account.upstream_account_id == Some(upstream_account_id))
            .and_then(|account| account.network_live_bucket.as_ref()),
    };
    let bucket = bucket?;
    let bucket_start = &bucket.bucket_start;
    base.points
        .iter()
        .position(|point| point.bucket_start == *bucket_start)
        .or_else(|| base.points.iter().position(|point| point.is_live_bucket))
        .map(|point_index| (point_index, bucket))
}

async fn wait_for_prompt_cache_reconcile_eligibility(
    gate: &DbPressureGate,
    observed_eligibility: u64,
    reason: DbPressureDenyReason,
) {
    match reason {
        DbPressureDenyReason::PressureCooldown { remaining_ms } => {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(remaining_ms.max(1))) => {}
                _ = gate.wait_for_eligibility_change(observed_eligibility) => {}
            }
        }
        DbPressureDenyReason::BackgroundBusy => {
            gate.wait_for_eligibility_change(observed_eligibility).await;
        }
    }
}

async fn run_server_push_topic_loop(
    hub: Arc<SubscriptionHub>,
    state: Arc<AppState>,
    topic_key: String,
    topic: SubscriptionTopic,
) {
    if matches!(
        topic,
        SubscriptionTopic::PromptCacheWindow { .. }
            | SubscriptionTopic::PromptCacheStickyWindow { .. }
            | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
    ) {
        let mut interval = tokio::time::interval(PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    hub.clear_server_push_task(&topic_key).await;
                    break;
                }
                _ = interval.tick() => {
                    if hub.stop_server_push_task_if_idle(&topic_key).await {
                        break;
                    }
                    let expiry_result = if matches!(
                        &topic,
                        SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    ) {
                        hub.expire_dashboard_working_conversations_projection(state.clone(), &topic_key)
                            .await
                    } else {
                        hub.expire_prompt_cache_topic_window(&topic_key).await
                    };
                    if let Err(err) = expiry_result {
                        warn!(?err, topic = %topic.name(), "failed to expire prompt cache topic window");
                    }
                    if !hub.prompt_cache_reconcile_required(&topic_key).await {
                        continue;
                    }
                    if !hub.begin_prompt_cache_topic_reconcile(&topic).await {
                        continue;
                    }
                    SubscriptionHub::spawn_prompt_cache_topic_reconcile(state.clone(), topic.clone());
                }
            }
        }
        return;
    }
    if !topic.is_closed_summary_topic() {
        let mut interval = tokio::time::interval(DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;

        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    hub.clear_server_push_task(&topic_key).await;
                    break;
                }
                _ = interval.tick() => {
                    if hub.stop_server_push_task_if_idle(&topic_key).await {
                        break;
                    }
                    if let Err(err) = hub
                        .refresh_topic_if_active(state.clone(), topic.clone(), true)
                        .await
                    {
                        warn!(?err, topic = %topic.name(), "failed to push legacy network recent topic cadence");
                    }
                }
            }
        }
        return;
    }

    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                hub.clear_server_push_task(&topic_key).await;
                break;
            }
            _ = tokio::time::sleep(subscription_calendar_rollover_delay(&topic)) => {
                if hub.stop_server_push_task_if_idle(&topic_key).await {
                    break;
                }
                if let Err(err) = hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    warn!(?err, topic = %topic.name(), "failed to refresh closed summary topic at calendar rollover");
                }
            }
        }
    }
}

pub(crate) fn spawn_subscription_broadcast_listener(state: Arc<AppState>) {
    let hub = state.subscription_hub.clone();
    let shutdown = state.shutdown.clone();
    let mut receiver = state.broadcaster.subscribe();
    let listener_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                item = receiver.recv() => {
                    match item {
                        Ok(payload) => hub.handle_internal_broadcast(listener_state.clone(), payload).await,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(skipped, "subscription mutation listener lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        }
    });
    spawn_runtime_mutation_router(state);
}

fn spawn_runtime_mutation_router(state: Arc<AppState>) {
    let hub = state.subscription_hub.clone();
    let bus = hub.runtime_mutation_bus();
    if !bus.claim_router() {
        return;
    }
    let shutdown = state.shutdown.clone();
    let mut receiver = bus.subscribe();
    tokio::spawn(async move {
        let mut last_sequence = 0_u64;
        loop {
            let first = tokio::select! {
                _ = shutdown.cancelled() => return,
                item = receiver.recv() => item,
            };
            let first = match first {
                Ok(first) => first,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    bus.record_router_lag();
                    bus.record_router_gap();
                    bus.record_cursor_recovery();
                    hub.mark_runtime_mutation_gap_and_recover(
                        state.clone(),
                        skipped,
                        "receiver_lagged",
                    )
                    .await;
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let mut batch = vec![first];
            let mut lagged = 0_u64;
            while batch.len() < RUNTIME_MUTATION_ROUTER_MAX_BATCH {
                match receiver.try_recv() {
                    Ok(mutation) => batch.push(mutation),
                    Err(broadcast::error::TryRecvError::Empty) => break,
                    Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                        lagged = lagged.saturating_add(skipped);
                        break;
                    }
                    Err(broadcast::error::TryRecvError::Closed) => break,
                }
            }
            if lagged > 0 {
                bus.record_router_lag();
                bus.record_router_gap();
                bus.record_cursor_recovery();
                hub.mark_runtime_mutation_gap_and_recover(state.clone(), lagged, "receiver_lagged")
                    .await;
                continue;
            }
            if runtime_mutation_batch_has_sequence_gap(&mut last_sequence, &batch) {
                bus.record_router_gap();
                bus.record_cursor_recovery();
                hub.mark_runtime_mutation_gap_and_recover(state.clone(), lagged, "cursor_gap")
                    .await;
                continue;
            }
            hub.handle_runtime_mutation_batch(state.clone(), batch)
                .await;
        }
    });
}

fn runtime_mutation_batch_has_sequence_gap(
    last_sequence: &mut u64,
    batch: &[SequencedRuntimeMutation],
) -> bool {
    let mut gap = false;
    for mutation in batch {
        if mutation.sequence != last_sequence.saturating_add(1) {
            gap = true;
        }
        *last_sequence = mutation.sequence;
    }
    gap
}

pub(crate) async fn topic_sse_stream(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubscriptionStreamQuery>,
) -> Result<Response, ApiError> {
    let descriptors = decode_topics_query(query.topics.as_deref())?;
    let resume = decode_resume_query(query.resume.as_deref(), &descriptors)?;
    let resume_count = resume.len();
    let mut live_receiver = state.subscription_hub.subscribe();
    let selected_topics = descriptors
        .iter()
        .map(SubscriptionTopic::from_descriptor)
        .collect::<Result<Vec<_>, _>>()?;
    let selected_topic_keys = selected_topics
        .iter()
        .map(SubscriptionTopic::cache_key)
        .collect::<Result<HashSet<_>, _>>()?;
    let selected_dashboard_topology_topic_names = selected_topics
        .iter()
        .map(SubscriptionTopic::name)
        .filter(|topic_name| {
            matches!(
                *topic_name,
                "dashboard.activity.current"
                    | "stats.summary.current"
                    | "dashboard.network-timeseries.window"
                    | "dashboard.network-recent.current"
                    | "dashboard.working-conversations.current"
                    | "stats.parallel-work.current"
                    | "stats.timeseries.open-window"
            )
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    #[cfg(test)]
    let dashboard_topology_observer_attempt = (query.reason.as_deref()
        == Some(DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON))
    .then_some(query.attempt)
    .flatten();
    let dashboard_topology_hub = state.subscription_hub.clone();
    let topic_lease = state
        .subscription_hub
        .register_topic_subscribers(&selected_topics)
        .await?;
    let prepared = state
        .subscription_hub
        .prepare_connection(state.clone(), descriptors, resume)
        .await?;
    if selected_topics.iter().any(|topic| {
        topic.uses_dashboard_activity_live_overlay()
            || topic.uses_summary_live_overlay()
            || topic.uses_timeseries_live_projection()
            || topic.uses_dashboard_network_live_snapshot()
    }) {
        ensure_dashboard_activity_live_snapshot_producer(state.as_ref());
    }
    tracing::info!(
        attempt = query.attempt,
        reason = query.reason.as_deref().unwrap_or("unknown"),
        topic_count = selected_topic_keys.len(),
        resume_count,
        init_outcomes = ?prepared.outcomes,
        "subscription connection prepared"
    );
    let PreparedSubscriptionConnection {
        initial,
        last_sent_cursors: last_seen_by_topic,
        outcomes: _,
    } = prepared;
    let runtime_projection_mode = state.proxy_runtime_invocations.mode();
    let server_push_topics = selected_topics
        .iter()
        .filter(|topic| topic.uses_server_push_cadence(runtime_projection_mode))
        .cloned()
        .collect::<Vec<_>>();
    let server_push_lease = state
        .subscription_hub
        .register_server_push_topics(state.clone(), server_push_topics)
        .await?;

    let initial_stream = stream::iter(initial.into_iter().flat_map(|prepared| {
        prepared
            .frame
            .event_chunks(prepared.kind)
            .map(Ok::<_, Infallible>)
    }));

    let live_stream = async_stream::stream! {
        let _topic_lease = topic_lease;
        let _server_push_lease = server_push_lease;
        let mut last_seen = last_seen_by_topic;
        let mut keep_alive = tokio::time::interval(Duration::from_secs(15));
        keep_alive.tick().await;
        loop {
            tokio::select! {
                _ = keep_alive.tick() => yield Ok::<_, Infallible>(Bytes::from_static(b":\n\n")),
                received = live_receiver.recv() => match received {
                    Ok(dispatch) => {
                        if !selected_topic_keys.contains(&dispatch.frame.topic_key) {
                            continue;
                        }
                        let previous_cursor = last_seen.get(&dispatch.frame.topic_key).copied().unwrap_or(0);
                        if dispatch.frame.cursor <= previous_cursor {
                            continue;
                        }
                        last_seen.insert(dispatch.frame.topic_key.clone(), dispatch.frame.cursor);
                        dashboard_topology_hub
                            .record_dashboard_topology_frame_delivery(&dispatch.frame);
                        #[cfg(test)]
                        if let Some(attempt) = dashboard_topology_observer_attempt {
                            dashboard_topology_hub
                                .record_dashboard_topology_sse_frame_delivery(attempt, &dispatch.frame)
                                .await;
                        }
                        for chunk in dispatch.frame.event_chunks(TopicFrameKind::Live) {
                            yield Ok::<_, Infallible>(chunk);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        dashboard_topology_hub.record_dashboard_topology_lag(
                            &selected_dashboard_topology_topic_names,
                            skipped,
                        );
                        warn!(skipped, "subscription live fanout lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    };

    let merged = initial_stream.chain(live_stream);
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(merged))
        .map_err(|err| ApiError::from(anyhow!(err)))
}

impl SubscriptionTopic {
    fn class(&self) -> SubscriptionTopicClass {
        match self {
            Self::DashboardActivityCurrent { range, .. } => {
                if range == "yesterday" {
                    SubscriptionTopicClass::ClosedSnapshot
                } else {
                    SubscriptionTopicClass::HotProjection
                }
            }
            Self::SummaryCurrent { window, .. } => {
                if matches!(window.as_str(), "yesterday" | "previous7d") {
                    SubscriptionTopicClass::ClosedSnapshot
                } else {
                    SubscriptionTopicClass::HotProjection
                }
            }
            Self::ParallelWorkCurrent { range, .. } | Self::TimeseriesOpenWindow { range, .. } => {
                if range == "yesterday" {
                    SubscriptionTopicClass::ClosedSnapshot
                } else {
                    SubscriptionTopicClass::HotProjection
                }
            }
            Self::DashboardNetworkTimeseriesWindow { .. }
            | Self::DashboardNetworkRecentCurrent
            | Self::DashboardWorkingConversationsCurrent { .. } => {
                SubscriptionTopicClass::HotProjection
            }
            Self::AppVersion
            | Self::QuotaCurrent
            | Self::InvocationWindow { .. }
            | Self::InvocationHistoryWindow { .. }
            | Self::InvocationHistoryOverview { .. }
            | Self::PromptCacheConversationBindingCurrent { .. }
            | Self::PromptCacheConversationOperationsWindow { .. }
            | Self::PromptCacheWindow { .. }
            | Self::PromptCacheStickyWindow { .. }
            | Self::ForwardProxyLive
            | Self::InvocationPoolAttempts { .. }
            | Self::ModelRoutingLive { .. } => SubscriptionTopicClass::BoundedColdHydrate,
        }
    }

    fn uses_server_push_cadence(&self, mode: RuntimeProjectionMode) -> bool {
        self.is_closed_summary_topic()
            || matches!(
                self,
                Self::PromptCacheWindow { .. }
                    | Self::PromptCacheStickyWindow { .. }
                    | Self::DashboardWorkingConversationsCurrent { .. }
            )
            || (mode == RuntimeProjectionMode::Legacy
                && matches!(self, Self::DashboardNetworkRecentCurrent))
    }

    fn uses_dashboard_activity_live_overlay(&self) -> bool {
        matches!(
            self,
            Self::DashboardActivityCurrent { range, .. } if range != "yesterday"
        )
    }

    fn uses_summary_live_overlay(&self) -> bool {
        matches!(
            self,
            Self::SummaryCurrent { window, .. }
                if !matches!(window.as_str(), "yesterday" | "previous7d")
        )
    }

    fn uses_timeseries_live_projection(&self) -> bool {
        matches!(self, Self::TimeseriesOpenWindow { range, .. } if range != "yesterday")
    }

    fn uses_summary_topic_refresh(&self) -> bool {
        self.uses_summary_live_overlay()
    }

    fn uses_conversation_overview_refresh(&self) -> bool {
        matches!(self, Self::InvocationHistoryOverview { .. })
    }

    fn is_unmigrated_dashboard_hot_projection(&self) -> bool {
        false
    }

    fn uses_parallel_work_live_projection(&self) -> bool {
        matches!(self, Self::ParallelWorkCurrent { range, .. } if range != "yesterday")
    }

    fn is_closed_dashboard_hot_snapshot(&self) -> bool {
        matches!(
            self,
            Self::ParallelWorkCurrent { range, .. } | Self::TimeseriesOpenWindow { range, .. }
                if range == "yesterday"
        )
    }

    fn uses_dashboard_network_live_snapshot(&self) -> bool {
        matches!(
            self,
            Self::DashboardNetworkTimeseriesWindow { .. } | Self::DashboardNetworkRecentCurrent
        )
    }

    fn runtime_topic_dependencies(&self) -> Vec<RuntimeTopicDependency> {
        match self {
            // These topics receive their revisions from the RuntimeProjectionHub directly, so
            // they intentionally do not create generic router work.
            Self::DashboardActivityCurrent { .. }
            | Self::DashboardNetworkTimeseriesWindow { .. }
            | Self::DashboardNetworkRecentCurrent
            | Self::SummaryCurrent { .. }
            | Self::AppVersion
            | Self::QuotaCurrent => Vec::new(),
            Self::PromptCacheWindow { .. } => vec![
                RuntimeTopicDependency::PromptCacheProjection,
                RuntimeTopicDependency::PromptCacheWindow,
            ],
            Self::PromptCacheStickyWindow { .. } => vec![
                RuntimeTopicDependency::PromptCacheProjection,
                RuntimeTopicDependency::PromptCacheStickyWindow,
            ],
            Self::DashboardWorkingConversationsCurrent { .. } => {
                vec![RuntimeTopicDependency::DashboardWorkingConversationsProjection]
            }
            Self::PromptCacheConversationBindingCurrent { scope }
            | Self::PromptCacheConversationOperationsWindow { scope, .. } => {
                vec![RuntimeTopicDependency::Binding(
                    scope.binding_key().to_string(),
                )]
            }
            Self::InvocationPoolAttempts { invoke_id } => {
                vec![RuntimeTopicDependency::Attempt(invoke_id.clone())]
            }
            Self::ModelRoutingLive { .. } => vec![RuntimeTopicDependency::ModelRouting],
            Self::InvocationHistoryWindow { scope } | Self::InvocationHistoryOverview { scope } => {
                match scope {
                    ConversationSubscriptionScope::PromptCacheKey(prompt_cache_key) => vec![
                        RuntimeTopicDependency::HistoryPromptCacheKey(prompt_cache_key.clone()),
                        RuntimeTopicDependency::StickyRoute(prompt_cache_key.clone()),
                    ],
                    ConversationSubscriptionScope::StickyKey { sticky_key, .. } => vec![
                        RuntimeTopicDependency::HistoryPromptCacheKey(sticky_key.clone()),
                        RuntimeTopicDependency::HistoryStickyKey(sticky_key.clone()),
                        RuntimeTopicDependency::StickyRoute(sticky_key.clone()),
                    ],
                }
            }
            Self::InvocationWindow { .. }
            | Self::TimeseriesOpenWindow { .. }
            | Self::ParallelWorkCurrent { .. }
            | Self::ForwardProxyLive => vec![RuntimeTopicDependency::Invocation],
        }
    }

    fn from_descriptor(descriptor: &SubscriptionTopicDescriptor) -> Result<Self, ApiError> {
        let topic = descriptor.topic.trim();
        let params = &descriptor.params;
        match topic {
            "app.version" => Ok(Self::AppVersion),
            "quota.current" => Ok(Self::QuotaCurrent),
            "dashboard.activity.current" => Ok(Self::DashboardActivityCurrent {
                range: param_or_default(params, "range", "today"),
                time_zone: param_or_default(params, "timeZone", SUBSCRIPTION_DEFAULT_TIME_ZONE),
                recent_limit: parse_i64_param(
                    params,
                    "recentLimit",
                    Some(SUBSCRIPTION_DEFAULT_DASHBOARD_RECENT_LIMIT),
                )?,
                include_accounts: parse_bool_param(params, "includeAccounts", Some(true))?,
                include_recent: parse_bool_param(params, "includeRecent", Some(true))?,
            }),
            "dashboard.network-timeseries.window" => Ok(Self::DashboardNetworkTimeseriesWindow {
                range: param_or_default(params, "range", "today"),
                time_zone: param_or_default(params, "timeZone", SUBSCRIPTION_DEFAULT_TIME_ZONE),
                upstream_account_id: parse_optional_i64_param(params, "upstreamAccountId")?,
            }),
            "dashboard.network-recent.current" => Ok(Self::DashboardNetworkRecentCurrent),
            "dashboard.working-conversations.current" => {
                let page_size =
                    normalize_prompt_cache_conversation_page_size(Some(parse_i64_param(
                        params,
                        "pageSize",
                        Some(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE),
                    )?))?
                    .unwrap_or(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE);
                let recent_invocation_limit =
                    normalize_prompt_cache_conversation_recent_invocation_limit(Some(
                        parse_i64_param(
                            params,
                            "recentInvocationLimit",
                            Some(SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT),
                        )?,
                    ))?
                    .unwrap_or(SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT);
                Ok(Self::DashboardWorkingConversationsCurrent {
                    page_size,
                    recent_invocation_limit,
                    blocked_binding_upstream_account_id: parse_optional_i64_param(
                        params,
                        "blockedBindingUpstreamAccountId",
                    )?,
                    blocked_binding_constraint_source:
                        parse_optional_blocked_binding_constraint_source_param(
                            params,
                            "blockedBindingConstraintSource",
                        )?,
                })
            }
            "invocations.window" => Ok(Self::InvocationWindow {
                limit: parse_i64_param(
                    params,
                    "limit",
                    Some(SUBSCRIPTION_DEFAULT_INVOCATION_LIMIT),
                )?,
                model: parse_optional_text_param(params, "model"),
                status: parse_optional_text_param(params, "status"),
            }),
            "invocation-history.window" => Ok(Self::InvocationHistoryWindow {
                scope: parse_conversation_subscription_scope(params)?,
            }),
            "invocation-history.overview" => Ok(Self::InvocationHistoryOverview {
                scope: parse_conversation_subscription_scope(params)?,
            }),
            "prompt-cache.conversation-binding.current" => {
                Ok(Self::PromptCacheConversationBindingCurrent {
                    scope: parse_conversation_subscription_scope(params)?,
                })
            }
            "prompt-cache.conversation-operations.window" => {
                Ok(Self::PromptCacheConversationOperationsWindow {
                    scope: parse_conversation_subscription_scope(params)?,
                    info_type: parse_optional_conversation_operation_info_type(params)?,
                })
            }
            "prompt-cache.window" => {
                let selection = parse_prompt_cache_selection(params)?;
                Ok(Self::PromptCacheWindow {
                    selection,
                    detail_level: parse_prompt_cache_detail_level(params)?,
                    recent_invocation_limit: parse_optional_i64_param(
                        params,
                        "recentInvocationLimit",
                    )?,
                })
            }
            "prompt-cache.sticky.window" => {
                let account_id = parse_required_i64_param(params, "accountId")?;
                Ok(Self::PromptCacheStickyWindow {
                    account_id,
                    selection: parse_sticky_selection(params)?,
                })
            }
            "stats.summary.current" => Ok(Self::SummaryCurrent {
                window: param_or_default(params, "window", "current"),
                time_zone: param_or_default(params, "timeZone", SUBSCRIPTION_DEFAULT_TIME_ZONE),
                limit: parse_optional_i64_param(params, "limit")?,
                upstream_account_id: parse_optional_i64_param(params, "upstreamAccountId")?,
            }),
            "stats.timeseries.open-window" => Ok(Self::TimeseriesOpenWindow {
                range: param_or_default(params, "range", "today"),
                time_zone: param_or_default(params, "timeZone", SUBSCRIPTION_DEFAULT_TIME_ZONE),
                bucket: parse_optional_text_param(params, "bucket"),
                settlement_hour: parse_optional_u8_param(params, "settlementHour")?,
                upstream_account_id: parse_optional_i64_param(params, "upstreamAccountId")?,
            }),
            "stats.parallel-work.current" => Ok(Self::ParallelWorkCurrent {
                range: param_or_default(params, "range", "current"),
                time_zone: param_or_default(params, "timeZone", SUBSCRIPTION_DEFAULT_TIME_ZONE),
                bucket: parse_optional_text_param(params, "bucket"),
                upstream_account_id: parse_optional_i64_param(params, "upstreamAccountId")?,
            }),
            "forward-proxy.live" => Ok(Self::ForwardProxyLive),
            "invocation.pool-attempts" => Ok(Self::InvocationPoolAttempts {
                invoke_id: parse_required_text_param(params, "invokeId")?,
            }),
            "pool.model-routing-live" => {
                let window = param_or_default(params, "window", "1h");
                if !matches!(window.as_str(), "15m" | "1h" | "6h" | "24h") {
                    return Err(ApiError::bad_request(anyhow!(
                        "model-routing window must be one of: 15m, 1h, 6h, 24h"
                    )));
                }
                let limit = parse_i64_param(params, "limit", Some(100))?;
                if !(1..=100).contains(&limit) {
                    return Err(ApiError::bad_request(anyhow!(
                        "model-routing limit must be between 1 and 100"
                    )));
                }
                let state = parse_optional_text_param(params, "state");
                if let Some(state) = state.as_deref()
                    && !matches!(state, "available" | "degraded" | "cooling_down")
                {
                    return Err(ApiError::bad_request(anyhow!(
                        "model-routing state must be available, degraded, or cooling_down"
                    )));
                }
                Ok(Self::ModelRoutingLive {
                    window,
                    model: parse_optional_text_param(params, "model"),
                    state,
                    limit,
                })
            }
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported subscription topic: {topic}"
            ))),
        }
    }

    fn descriptor(&self) -> SubscriptionTopicDescriptor {
        match self {
            Self::AppVersion => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::QuotaCurrent => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::DashboardActivityCurrent {
                range,
                time_zone,
                recent_limit,
                include_accounts,
                include_recent,
            } => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: btree_map_from_pairs([
                    ("range", range.clone()),
                    ("timeZone", time_zone.clone()),
                    ("recentLimit", recent_limit.to_string()),
                    ("includeAccounts", include_accounts.to_string()),
                    ("includeRecent", include_recent.to_string()),
                ]),
            },
            Self::DashboardNetworkTimeseriesWindow {
                range,
                time_zone,
                upstream_account_id,
            } => {
                let mut params = btree_map_from_pairs([
                    ("range", range.clone()),
                    ("timeZone", time_zone.clone()),
                ]);
                insert_optional_param(
                    &mut params,
                    "upstreamAccountId",
                    upstream_account_id.map(|value| value.to_string()),
                );
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::DashboardNetworkRecentCurrent => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::DashboardWorkingConversationsCurrent {
                page_size,
                recent_invocation_limit,
                blocked_binding_upstream_account_id,
                blocked_binding_constraint_source,
            } => {
                let mut params = btree_map_from_pairs([
                    ("pageSize", page_size.to_string()),
                    ("recentInvocationLimit", recent_invocation_limit.to_string()),
                ]);
                insert_optional_param(
                    &mut params,
                    "blockedBindingUpstreamAccountId",
                    blocked_binding_upstream_account_id.map(|value| value.to_string()),
                );
                insert_optional_param(
                    &mut params,
                    "blockedBindingConstraintSource",
                    blocked_binding_constraint_source.map(|value| match value {
                        BlockedBindingConstraintSource::UpstreamAccountBinding => {
                            "upstreamAccountBinding".to_string()
                        }
                        BlockedBindingConstraintSource::EncryptedSessionOwner => {
                            "encryptedSessionOwner".to_string()
                        }
                    }),
                );
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::InvocationWindow {
                limit,
                model,
                status,
            } => {
                let mut params = btree_map_from_pairs([("limit", limit.to_string())]);
                insert_optional_param(&mut params, "model", model.clone());
                insert_optional_param(&mut params, "status", status.clone());
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::InvocationHistoryWindow { scope }
            | Self::InvocationHistoryOverview { scope }
            | Self::PromptCacheConversationBindingCurrent { scope } => {
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params: scope.descriptor_params(),
                }
            }
            Self::PromptCacheConversationOperationsWindow { scope, info_type } => {
                let mut params = scope.descriptor_params();
                insert_optional_param(&mut params, "infoType", info_type.clone());
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::PromptCacheWindow {
                selection,
                detail_level,
                recent_invocation_limit,
            } => {
                let mut params = prompt_cache_selection_params(*selection);
                params.insert(
                    "detail".to_string(),
                    match detail_level {
                        PromptCacheConversationDetailLevel::Full => "full".to_string(),
                        PromptCacheConversationDetailLevel::Compact => "compact".to_string(),
                    },
                );
                if let Some(limit) = recent_invocation_limit {
                    params.insert("recentInvocationLimit".to_string(), limit.to_string());
                }
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::PromptCacheStickyWindow {
                account_id,
                selection,
            } => {
                let mut params =
                    BTreeMap::from([("accountId".to_string(), account_id.to_string())]);
                match selection {
                    AccountStickyKeySelection::Count(limit) => {
                        params.insert("limit".to_string(), limit.to_string());
                    }
                    AccountStickyKeySelection::ActivityWindow(hours) => {
                        params.insert("activityHours".to_string(), hours.to_string());
                    }
                }
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::SummaryCurrent {
                window,
                time_zone,
                limit,
                upstream_account_id,
            } => {
                let mut params = btree_map_from_pairs([
                    ("window", window.clone()),
                    ("timeZone", time_zone.clone()),
                ]);
                insert_optional_param(&mut params, "limit", limit.map(|value| value.to_string()));
                insert_optional_param(
                    &mut params,
                    "upstreamAccountId",
                    upstream_account_id.map(|value| value.to_string()),
                );
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::TimeseriesOpenWindow {
                range,
                time_zone,
                bucket,
                settlement_hour,
                upstream_account_id,
            } => {
                let mut params = btree_map_from_pairs([
                    ("range", range.clone()),
                    ("timeZone", time_zone.clone()),
                ]);
                insert_optional_param(&mut params, "bucket", bucket.clone());
                insert_optional_param(
                    &mut params,
                    "settlementHour",
                    settlement_hour.map(|value| value.to_string()),
                );
                insert_optional_param(
                    &mut params,
                    "upstreamAccountId",
                    upstream_account_id.map(|value| value.to_string()),
                );
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::ParallelWorkCurrent {
                range,
                time_zone,
                bucket,
                upstream_account_id,
            } => {
                let mut params = btree_map_from_pairs([
                    ("range", range.clone()),
                    ("timeZone", time_zone.clone()),
                ]);
                insert_optional_param(&mut params, "bucket", bucket.clone());
                insert_optional_param(
                    &mut params,
                    "upstreamAccountId",
                    upstream_account_id.map(|value| value.to_string()),
                );
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
            Self::ForwardProxyLive => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::InvocationPoolAttempts { invoke_id } => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::from([("invokeId".to_string(), invoke_id.clone())]),
            },
            Self::ModelRoutingLive {
                window,
                model,
                state,
                limit,
            } => {
                let mut params = btree_map_from_pairs([
                    ("window", window.clone()),
                    ("limit", limit.to_string()),
                ]);
                insert_optional_param(&mut params, "model", model.clone());
                insert_optional_param(&mut params, "state", state.clone());
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::AppVersion => "app.version",
            Self::QuotaCurrent => "quota.current",
            Self::DashboardActivityCurrent { .. } => "dashboard.activity.current",
            Self::DashboardNetworkTimeseriesWindow { .. } => "dashboard.network-timeseries.window",
            Self::DashboardNetworkRecentCurrent => "dashboard.network-recent.current",
            Self::DashboardWorkingConversationsCurrent { .. } => {
                "dashboard.working-conversations.current"
            }
            Self::InvocationWindow { .. } => "invocations.window",
            Self::InvocationHistoryWindow { .. } => "invocation-history.window",
            Self::InvocationHistoryOverview { .. } => "invocation-history.overview",
            Self::PromptCacheConversationBindingCurrent { .. } => {
                "prompt-cache.conversation-binding.current"
            }
            Self::PromptCacheConversationOperationsWindow { .. } => {
                "prompt-cache.conversation-operations.window"
            }
            Self::PromptCacheWindow { .. } => "prompt-cache.window",
            Self::PromptCacheStickyWindow { .. } => "prompt-cache.sticky.window",
            Self::SummaryCurrent { .. } => "stats.summary.current",
            Self::TimeseriesOpenWindow { .. } => "stats.timeseries.open-window",
            Self::ParallelWorkCurrent { .. } => "stats.parallel-work.current",
            Self::ForwardProxyLive => "forward-proxy.live",
            Self::InvocationPoolAttempts { .. } => "invocation.pool-attempts",
            Self::ModelRoutingLive { .. } => "pool.model-routing-live",
        }
    }

    fn schema_epoch(&self) -> String {
        match self {
            Self::AppVersion => "app.version/v1".to_string(),
            Self::QuotaCurrent => "quota.current/v1".to_string(),
            Self::DashboardActivityCurrent { .. } => "dashboard.activity.current/v2".to_string(),
            Self::DashboardNetworkTimeseriesWindow { .. } => {
                "dashboard.network-timeseries.window/v1".to_string()
            }
            Self::DashboardNetworkRecentCurrent => {
                "dashboard.network-recent.current/v1".to_string()
            }
            Self::DashboardWorkingConversationsCurrent { .. } => {
                "dashboard.working-conversations.current/v1".to_string()
            }
            Self::InvocationWindow { .. } => "invocations.window/v1".to_string(),
            Self::InvocationHistoryWindow { .. } => "invocation-history.window/v1".to_string(),
            Self::InvocationHistoryOverview { .. } => "invocation-history.overview/v1".to_string(),
            Self::PromptCacheConversationBindingCurrent { .. } => {
                "prompt-cache.conversation-binding.current/v1".to_string()
            }
            Self::PromptCacheConversationOperationsWindow { .. } => {
                "prompt-cache.conversation-operations.window/v1".to_string()
            }
            Self::PromptCacheWindow { .. } => "prompt-cache.window/v1".to_string(),
            Self::PromptCacheStickyWindow { .. } => "prompt-cache.sticky.window/v1".to_string(),
            Self::SummaryCurrent { .. } => "stats.summary.current/v1".to_string(),
            Self::TimeseriesOpenWindow { .. } => "stats.timeseries.open-window/v1".to_string(),
            Self::ParallelWorkCurrent { .. } => "stats.parallel-work.current/v1".to_string(),
            Self::ForwardProxyLive => "forward-proxy.live/v1".to_string(),
            Self::InvocationPoolAttempts { .. } => "invocation.pool-attempts/v1".to_string(),
            Self::ModelRoutingLive { .. } => "pool.model-routing-live/v1".to_string(),
        }
    }

    fn cache_key(&self) -> Result<String, ApiError> {
        serde_json::to_string(&self.descriptor()).map_err(ApiError::from)
    }

    fn is_closed_summary_topic(&self) -> bool {
        matches!(
            self,
            Self::SummaryCurrent { window, .. }
                if matches!(window.as_str(), "yesterday" | "previous7d")
        )
    }

    fn is_affected_by_runtime_mutation(&self, mutation: &RuntimeMutation) -> bool {
        match mutation {
            RuntimeMutation::Invocation(mutation) => {
                if self.is_closed_summary_topic() || self.is_closed_dashboard_hot_snapshot() {
                    return false;
                }
                match self {
                    Self::InvocationHistoryWindow { scope }
                    | Self::InvocationHistoryOverview { scope } => {
                        scope.matches_runtime_mutation(mutation)
                    }
                    Self::DashboardActivityCurrent { .. }
                    | Self::DashboardNetworkTimeseriesWindow { .. }
                    | Self::DashboardNetworkRecentCurrent
                    | Self::DashboardWorkingConversationsCurrent { .. }
                    | Self::InvocationWindow { .. }
                    | Self::SummaryCurrent { .. }
                    | Self::TimeseriesOpenWindow { .. }
                    | Self::ParallelWorkCurrent { .. }
                    | Self::ForwardProxyLive => true,
                    Self::AppVersion
                    | Self::QuotaCurrent
                    | Self::PromptCacheConversationBindingCurrent { .. }
                    | Self::PromptCacheConversationOperationsWindow { .. }
                    | Self::PromptCacheWindow { .. }
                    | Self::PromptCacheStickyWindow { .. }
                    | Self::InvocationPoolAttempts { .. }
                    | Self::ModelRoutingLive { .. } => false,
                }
            }
            RuntimeMutation::AttemptChanged { invoke_id } => matches!(
                self,
                Self::InvocationPoolAttempts { invoke_id: current } if current == invoke_id
            ),
            RuntimeMutation::ModelRoutingChanged => matches!(self, Self::ModelRoutingLive { .. }),
            RuntimeMutation::PromptCacheBindingChanged { prompt_cache_key } => matches!(
                self,
                Self::PromptCacheConversationBindingCurrent { scope }
                    | Self::PromptCacheConversationOperationsWindow { scope, .. }
                    if scope.binding_key() == prompt_cache_key
            ),
            RuntimeMutation::StickyRouteChanged {
                sticky_key,
                previous_upstream_account_id,
                upstream_account_id,
            } => match self {
                Self::InvocationHistoryWindow { scope }
                | Self::InvocationHistoryOverview { scope } => scope.matches_sticky_route_change(
                    sticky_key,
                    *previous_upstream_account_id,
                    *upstream_account_id,
                ),
                _ => false,
            },
        }
    }

    fn is_affected_by(&self, payload: &BroadcastPayload) -> bool {
        if self.is_closed_summary_topic()
            && matches!(payload, BroadcastPayload::DashboardActivityLive { .. })
        {
            return false;
        }

        match payload {
            BroadcastPayload::DashboardActivityLive { .. } => {
                matches!(
                    self,
                    Self::DashboardActivityCurrent { .. } | Self::SummaryCurrent { .. }
                )
            }
            BroadcastPayload::DashboardNetworkSlice { .. } => matches!(
                self,
                Self::DashboardActivityCurrent { .. }
                    | Self::DashboardNetworkTimeseriesWindow { .. }
            ),
            BroadcastPayload::DashboardCurrentSlice { .. }
            | BroadcastPayload::DashboardTerminalSlice { .. } => false,
            BroadcastPayload::Quota { .. } => matches!(self, Self::QuotaCurrent),
            BroadcastPayload::Version { .. } => matches!(self, Self::AppVersion),
            #[cfg(test)]
            BroadcastPayload::Records { .. }
            | BroadcastPayload::PoolAttempts { .. }
            | BroadcastPayload::PromptCacheConversationChanged { .. }
            | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. } => false,
        }
    }

    async fn build_cached_payload(
        &self,
        state: Arc<AppState>,
    ) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
        if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
            match self {
                Self::DashboardActivityCurrent {
                    range,
                    time_zone,
                    recent_limit,
                    include_accounts,
                    include_recent,
                } if range != "yesterday" => {
                    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
                    let source_scope = resolve_default_source_scope(&state.pool).await?;
                    let base = build_dashboard_activity_topic_materialized_base(
                        state.as_ref(),
                        &DashboardActivityQuery {
                            range: range.clone(),
                            recent_limit: Some(*recent_limit),
                            time_zone: Some(time_zone.clone()),
                            include_accounts: *include_accounts,
                            include_recent: Some(*include_recent),
                        },
                    )
                    .await?;
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::Activity {
                            base: Arc::new(StdMutex::new(DashboardActivityMaterializerState::new(
                                base,
                            ))),
                            reporting_tz,
                            source_scope,
                        },
                    ));
                }
                Self::SummaryCurrent {
                    window,
                    time_zone,
                    limit,
                    upstream_account_id,
                } if !matches!(window.as_str(), "yesterday" | "previous7d") => {
                    let query = SummaryQuery {
                        window: Some(window.clone()),
                        limit: *limit,
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    };
                    let summary_window =
                        parse_summary_window(&query, state.config.list_limit_max as i64)?;
                    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
                    let source_scope = resolve_default_source_scope(&state.pool).await?;
                    let SummaryTopicTerminalConsistentBase {
                        mut response,
                        pending_terminal_deltas,
                        terminal_sequence,
                    } = build_summary_topic_terminal_consistent_base(state.as_ref(), &query)
                        .await?;
                    let mut replayed_terminal_sequence = 0;
                    apply_dashboard_terminal_slice_to_summary_response(
                        &mut response,
                        &mut replayed_terminal_sequence,
                        &summary_window,
                        reporting_tz,
                        source_scope,
                        *upstream_account_id,
                        &DashboardTerminalProjectionSlice {
                            revision: 0,
                            deltas: pending_terminal_deltas,
                        },
                    );
                    let range_start =
                        summary_window_range(&summary_window, reporting_tz, Utc::now())?
                            .map(|(start, _)| start);
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::Summary {
                            base: Arc::new(StdMutex::new(DashboardSummaryMaterializerState::new(
                                response,
                                terminal_sequence,
                                range_start,
                            ))),
                            window: summary_window,
                            reporting_tz,
                            source_scope,
                            upstream_account_id: *upstream_account_id,
                        },
                    ));
                }
                Self::TimeseriesOpenWindow {
                    range,
                    time_zone,
                    bucket,
                    settlement_hour,
                    upstream_account_id,
                } if range != "yesterday" => {
                    let base = TimeseriesTopicMaterializedBase::build(
                        state.as_ref(),
                        &TimeseriesQuery {
                            range: range.clone(),
                            bucket: bucket.clone(),
                            settlement_hour: *settlement_hour,
                            time_zone: Some(time_zone.clone()),
                            upstream_account_id: *upstream_account_id,
                        },
                    )
                    .await?;
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::Timeseries {
                            base: Arc::new(StdMutex::new(base)),
                            runtime: state.proxy_runtime_invocations.clone(),
                        },
                    ));
                }
                Self::DashboardNetworkTimeseriesWindow {
                    range,
                    time_zone,
                    upstream_account_id,
                } => {
                    let Json(response) = fetch_dashboard_network_timeseries(
                        State(state),
                        Query(DashboardNetworkTimeseriesQuery {
                            range: range.clone(),
                            time_zone: Some(time_zone.clone()),
                            upstream_account_id: *upstream_account_id,
                        }),
                    )
                    .await?;
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::NetworkTimeseries {
                            base: Arc::new(response),
                            upstream_account_id: *upstream_account_id,
                        },
                    ));
                }
                Self::DashboardNetworkRecentCurrent => {
                    let Json(response) = fetch_dashboard_network_recent(
                        State(state),
                        Query(DashboardRecentNetworkWindowQuery::default()),
                    )
                    .await?;
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::NetworkRecent {
                            base: Arc::new(response),
                        },
                    ));
                }
                Self::ParallelWorkCurrent {
                    range,
                    time_zone,
                    bucket,
                    upstream_account_id,
                } if range != "yesterday" => {
                    let base = build_dashboard_parallel_work_materializer_state(
                        &state,
                        ParallelWorkStatsQuery {
                            range: range.clone(),
                            bucket: bucket.clone(),
                            time_zone: Some(time_zone.clone()),
                            upstream_account_id: *upstream_account_id,
                        },
                    )
                    .await?;
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::ParallelWork {
                            base: Arc::new(StdMutex::new(base)),
                        },
                    ));
                }
                Self::DashboardWorkingConversationsCurrent {
                    page_size,
                    recent_invocation_limit,
                    blocked_binding_upstream_account_id,
                    blocked_binding_constraint_source,
                } => {
                    let blocked_binding_filter = PromptCacheConversationBlockedBindingFilter {
                        upstream_account_id: *blocked_binding_upstream_account_id,
                        constraint_source: *blocked_binding_constraint_source,
                    };
                    let blocked_binding_filter = blocked_binding_filter
                        .is_active()
                        .then_some(blocked_binding_filter);
                    let response = build_prompt_cache_conversations_response_for_request(
                        state.as_ref(),
                        PromptCacheConversationsRequest {
                            selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                                SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                            ),
                            detail_level: PromptCacheConversationDetailLevel::Full,
                            recent_invocation_limit: Some(*recent_invocation_limit),
                            page_size: Some(*page_size),
                            cursor: None,
                            snapshot_at: None,
                            blocked_binding_filter: blocked_binding_filter.clone(),
                        },
                    )
                    .await?;
                    return Ok(BuiltSubscriptionTopicPayload::Dashboard(
                        DashboardTopicMaterializer::WorkingConversations {
                            state: Arc::new(StdMutex::new(
                                DashboardWorkingConversationsMaterializerState::new(
                                    response,
                                    *page_size,
                                    *recent_invocation_limit,
                                    blocked_binding_filter,
                                ),
                            )),
                        },
                    ));
                }
                _ => {}
            }
        }

        Ok(BuiltSubscriptionTopicPayload::Json(
            self.build_payload(state).await?,
        ))
    }

    async fn build_payload(&self, state: Arc<AppState>) -> Result<Value, ApiError> {
        match self {
            Self::AppVersion => {
                let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
                Ok(serde_json::to_value(VersionResponse { backend, frontend })?)
            }
            Self::QuotaCurrent => {
                let Json(snapshot) = latest_quota_snapshot(State(state)).await?;
                Ok(serde_json::to_value(snapshot)?)
            }
            Self::DashboardActivityCurrent {
                range,
                time_zone,
                recent_limit,
                include_accounts,
                include_recent,
            } => {
                let Json(response) = fetch_dashboard_activity(
                    State(state),
                    Query(DashboardActivityQuery {
                        range: range.clone(),
                        recent_limit: Some(*recent_limit),
                        time_zone: Some(time_zone.clone()),
                        include_accounts: *include_accounts,
                        include_recent: Some(*include_recent),
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::DashboardNetworkTimeseriesWindow {
                range,
                time_zone,
                upstream_account_id,
            } => {
                let Json(response) = fetch_dashboard_network_timeseries(
                    State(state),
                    Query(DashboardNetworkTimeseriesQuery {
                        range: range.clone(),
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::DashboardNetworkRecentCurrent => {
                let Json(response) = fetch_dashboard_network_recent(
                    State(state),
                    Query(DashboardRecentNetworkWindowQuery::default()),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::DashboardWorkingConversationsCurrent {
                page_size,
                recent_invocation_limit,
                blocked_binding_upstream_account_id,
                blocked_binding_constraint_source,
            } => {
                let Json(response) = fetch_prompt_cache_conversations(
                    State(state),
                    Query(PromptCacheConversationsQuery {
                        limit: None,
                        activity_hours: None,
                        activity_minutes: Some(
                            SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                        ),
                        page_size: Some(*page_size),
                        cursor: None,
                        snapshot_at: None,
                        detail: Some("full".to_string()),
                        recent_invocation_limit: Some(*recent_invocation_limit),
                        blocked_binding_upstream_account_id: *blocked_binding_upstream_account_id,
                        blocked_binding_constraint_source: blocked_binding_constraint_source.map(
                            |value| match value {
                                BlockedBindingConstraintSource::UpstreamAccountBinding => {
                                    "upstreamAccountBinding".to_string()
                                }
                                BlockedBindingConstraintSource::EncryptedSessionOwner => {
                                    "encryptedSessionOwner".to_string()
                                }
                            },
                        ),
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::InvocationWindow {
                limit,
                model,
                status,
            } => {
                let Json(response) = list_invocations(
                    State(state),
                    Query(ListQuery {
                        limit: Some(*limit),
                        page: Some(1),
                        page_size: Some(*limit),
                        snapshot_id: None,
                        anchor_id: None,
                        sort_by: Some("occurredAt".to_string()),
                        sort_order: Some("desc".to_string()),
                        range_preset: None,
                        from: None,
                        to: None,
                        model: model.clone(),
                        status: status.clone(),
                        proxy: None,
                        endpoint: None,
                        request_id: None,
                        failure_class: None,
                        failure_kind: None,
                        prompt_cache_key: None,
                        sticky_key: None,
                        upstream_scope: None,
                        upstream_account_id: None,
                        ..Default::default()
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::InvocationHistoryWindow { scope } => {
                let Json(response) = list_invocations(
                    State(state),
                    Query(scope.list_query(1, SUBSCRIPTION_CONVERSATION_HISTORY_LIMIT, None)),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::InvocationHistoryOverview { scope } => {
                let runtime_overlay_records = runtime_overlay_snapshot(state.as_ref());
                let overview = fetch_invocation_history_overview_with_runtime_overlay(
                    state.clone(),
                    scope.list_query(1, SUBSCRIPTION_CONVERSATION_HISTORY_LIMIT, None),
                    runtime_overlay_records,
                    SUBSCRIPTION_CONVERSATION_OVERVIEW_MAX_RECORDS,
                )
                .await?;
                Ok(serde_json::to_value(overview)?)
            }
            Self::PromptCacheConversationBindingCurrent { scope } => Ok(serde_json::to_value(
                load_prompt_cache_conversation_binding_response_for_key(
                    state.as_ref(),
                    scope.binding_key().to_string(),
                )
                .await?,
            )?),
            Self::PromptCacheConversationOperationsWindow { scope, info_type } => {
                let Json(response) = list_prompt_cache_conversation_operation_events(
                    State(state),
                    AxumPath(scope.binding_key().to_string()),
                    Query(ListPromptCacheConversationOperationEventsQuery {
                        page: Some(1),
                        page_size: Some(SUBSCRIPTION_CONVERSATION_OPERATION_LIMIT),
                        info_type: info_type.clone(),
                        routing_scope: None,
                        routing_model: None,
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::PromptCacheWindow {
                selection,
                detail_level,
                recent_invocation_limit,
            } => {
                let (limit, activity_hours, activity_minutes) = match selection {
                    PromptCacheConversationSelection::Count(limit) => (Some(*limit), None, None),
                    PromptCacheConversationSelection::ActivityWindowHours(hours) => {
                        (None, Some(*hours), None)
                    }
                    PromptCacheConversationSelection::ActivityWindowMinutes(minutes) => {
                        (None, None, Some(*minutes))
                    }
                };
                let Json(response) = fetch_prompt_cache_conversations(
                    State(state),
                    Query(PromptCacheConversationsQuery {
                        limit,
                        activity_hours,
                        activity_minutes,
                        page_size: None,
                        cursor: None,
                        snapshot_at: None,
                        detail: Some(match detail_level {
                            PromptCacheConversationDetailLevel::Full => "full".to_string(),
                            PromptCacheConversationDetailLevel::Compact => "compact".to_string(),
                        }),
                        recent_invocation_limit: *recent_invocation_limit,
                        blocked_binding_upstream_account_id: None,
                        blocked_binding_constraint_source: None,
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::PromptCacheStickyWindow {
                account_id,
                selection,
            } => Ok(serde_json::to_value(
                build_account_sticky_keys_response(&state.pool, *account_id, *selection)
                    .await
                    .map_err(ApiError::from)?,
            )?),
            Self::SummaryCurrent {
                window,
                time_zone,
                limit,
                upstream_account_id,
            } => {
                let response = load_summary_response_from_query(
                    state.as_ref(),
                    &SummaryQuery {
                        window: Some(window.clone()),
                        limit: *limit,
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    },
                    SummaryBuildRoute::Topic,
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::TimeseriesOpenWindow {
                range,
                time_zone,
                bucket,
                settlement_hour,
                upstream_account_id,
            } => {
                let Json(response) = fetch_timeseries(
                    State(state),
                    Query(TimeseriesQuery {
                        range: range.clone(),
                        bucket: bucket.clone(),
                        settlement_hour: *settlement_hour,
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    }),
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::ParallelWorkCurrent {
                range,
                time_zone,
                bucket,
                upstream_account_id,
            } => {
                let response = load_parallel_work_stats_response(
                    &state,
                    ParallelWorkStatsQuery {
                        range: range.clone(),
                        bucket: bucket.clone(),
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    },
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::ForwardProxyLive => {
                let Json(response) = fetch_forward_proxy_live_stats(State(state)).await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::InvocationPoolAttempts { invoke_id } => {
                let Json(response) =
                    fetch_invocation_pool_attempts(State(state), AxumPath(invoke_id.clone()))
                        .await?;
                Ok(serde_json::to_value(response)?)
            }
            Self::ModelRoutingLive {
                window,
                model,
                state: route_state,
                limit,
            } => {
                let Json(response) = get_model_routing_live(
                    State(state),
                    Query(ModelRoutingLiveQuery {
                        window: Some(window.clone()),
                        model: model.clone(),
                        state: route_state.clone(),
                        limit: Some(*limit as usize),
                    }),
                )
                .await
                .map_err(|(_status, message)| ApiError::bad_request(anyhow!(message)))?;
                Ok(serde_json::to_value(response)?)
            }
        }
    }
}

impl RuntimeMutation {
    fn topic_dependencies(&self) -> Vec<RuntimeTopicDependency> {
        match self {
            Self::Invocation(mutation) => {
                let mut dependencies = vec![
                    RuntimeTopicDependency::Invocation,
                    RuntimeTopicDependency::PromptCacheProjection,
                ];
                if let Some(prompt_cache_key) = mutation.prompt_cache_key.as_ref() {
                    dependencies.push(RuntimeTopicDependency::HistoryPromptCacheKey(
                        prompt_cache_key.clone(),
                    ));
                }
                if let Some(sticky_key) = mutation.sticky_key.as_ref() {
                    dependencies.push(RuntimeTopicDependency::HistoryStickyKey(sticky_key.clone()));
                }
                dependencies
            }
            Self::AttemptChanged { invoke_id } => {
                vec![RuntimeTopicDependency::Attempt(invoke_id.clone())]
            }
            Self::ModelRoutingChanged => vec![RuntimeTopicDependency::ModelRouting],
            Self::PromptCacheBindingChanged { prompt_cache_key } => {
                vec![RuntimeTopicDependency::Binding(prompt_cache_key.clone())]
            }
            Self::StickyRouteChanged { sticky_key, .. } => vec![
                RuntimeTopicDependency::StickyRoute(sticky_key.clone()),
                RuntimeTopicDependency::PromptCacheStickyWindow,
            ],
        }
    }
}

fn decode_topics_query(raw: Option<&str>) -> Result<Vec<SubscriptionTopicDescriptor>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    decode_query_json(raw, "topics")
}

fn decode_resume_query(
    raw: Option<&str>,
    descriptors: &[SubscriptionTopicDescriptor],
) -> Result<Vec<SubscriptionResumeCursor>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let query_items = decode_query_json::<Vec<SubscriptionResumeCursorQuery>>(raw, "resume")?;
    query_items
        .into_iter()
        .map(|item| match item {
            SubscriptionResumeCursorQuery::Legacy(cursor) => Ok(cursor),
            SubscriptionResumeCursorQuery::Compact(cursor) => {
                let descriptor = descriptors.get(cursor.topic_index).ok_or_else(|| {
                    ApiError::bad_request(anyhow!(
                        "resume topicIndex out of range: {}",
                        cursor.topic_index
                    ))
                })?;
                let topic_key = SubscriptionTopic::from_descriptor(descriptor)?.cache_key()?;
                Ok(SubscriptionResumeCursor {
                    topic_key,
                    cursor: cursor.cursor,
                    schema_epoch: cursor.schema_epoch,
                })
            }
        })
        .collect()
}

fn decode_query_json<T: DeserializeOwned>(raw: &str, field: &str) -> Result<T, ApiError> {
    if raw.starts_with('[') || raw.starts_with('{') {
        return serde_json::from_str(raw).map_err(ApiError::from);
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(raw))
        .map_err(|err| ApiError::bad_request(anyhow!("invalid {field} payload: {err}")))?;
    serde_json::from_slice(&bytes).map_err(ApiError::from)
}

fn serialize_topic_frame(
    descriptor: SubscriptionTopicDescriptor,
    topic_key: String,
    schema_epoch: String,
    cursor: u64,
    payload_bytes: Vec<u8>,
) -> Result<SerializedTopicFrame, ApiError> {
    let mut hasher = DefaultHasher::new();
    payload_bytes.hash(&mut hasher);
    let fingerprint = hasher.finish();
    let descriptor_json = serde_json::to_string(&descriptor)?;
    let topic_key_json = serde_json::to_string(&topic_key)?;
    let schema_epoch_json = serde_json::to_string(&schema_epoch)?;
    let envelope_metadata_bytes = Bytes::from(format!(
        r#"","topic":{descriptor_json},"topicKey":{topic_key_json},"schemaEpoch":{schema_epoch_json},"cursor":{cursor},"payload":"#
    ));
    Ok(SerializedTopicFrame {
        topic_key,
        schema_epoch,
        cursor,
        descriptor,
        fingerprint,
        payload_bytes: Bytes::from(payload_bytes),
        envelope_metadata_bytes,
    })
}

fn reuse_unchanged_cached_topic(
    existing: &mut CachedSubscriptionTopic,
    serialized_payload: &[u8],
) -> Option<CachedSubscriptionTopic> {
    if existing.snapshot_frame.payload_bytes.as_ref() != serialized_payload {
        return None;
    }
    existing.dirty = false;
    existing.refresh_scheduled = false;
    existing.runtime_topic_recovery_retry_at = None;
    // A successful Prompt Cache baseline can serialize identically to last-good. It still
    // proves reconciliation completed, so do not keep scheduling the same cold hydration.
    existing.prompt_cache_reconcile_required = false;
    existing.prompt_cache_pressure_deferred = false;
    existing.prompt_cache_pending_key_hydrations.clear();
    existing.prompt_cache_candidate_refill_required = false;
    existing.prompt_cache_key_hydration_scheduled = false;
    existing.snapshot_built_at = Instant::now();
    Some(existing.clone())
}

fn finish_prompt_cache_baseline_reuse(
    existing: &mut CachedSubscriptionTopic,
    build: &PromptCacheBaselineBuild,
    applied_terminal_ids: &HashSet<String>,
) {
    existing.prompt_cache_full_hydration_count =
        existing.prompt_cache_full_hydration_count.saturating_add(1);
    existing.prompt_cache_baseline_at = Some(Instant::now());
    existing.prompt_cache_baseline_row_id = build.baseline_row_id;
    existing.prompt_cache_response_source = "database_reconcile";
    existing.prompt_cache_applied_terminal_ids = applied_terminal_ids.clone();
}

fn prune_replay_window(events: &mut VecDeque<ReplayableTopicEvent>, total_bytes: &mut usize) {
    let cutoff = Utc::now() - ChronoDuration::seconds(SUBSCRIPTION_REPLAY_WINDOW_SECS);
    while let Some(front) = events.front() {
        let should_drop = events.len() > SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC
            || *total_bytes > SUBSCRIPTION_REPLAY_MAX_BYTES_PER_TOPIC
            || front.emitted_at < cutoff;
        if !should_drop {
            break;
        }
        if let Some(removed) = events.pop_front() {
            *total_bytes = total_bytes.saturating_sub(removed.bytes);
        }
    }
}

fn btree_map_from_pairs<const N: usize>(pairs: [(&str, String); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn insert_optional_param(params: &mut BTreeMap<String, String>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        params.insert(key.to_string(), value);
    }
}

fn param_or_default(params: &BTreeMap<String, String>, key: &str, default: &str) -> String {
    params
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn parse_optional_text_param(params: &BTreeMap<String, String>, key: &str) -> Option<String> {
    params
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_required_text_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<String, ApiError> {
    parse_optional_text_param(params, key).ok_or_else(|| {
        ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
    })
}

fn parse_required_i64_param(params: &BTreeMap<String, String>, key: &str) -> Result<i64, ApiError> {
    parse_optional_i64_param(params, key)?.ok_or_else(|| {
        ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
    })
}

fn parse_i64_param(
    params: &BTreeMap<String, String>,
    key: &str,
    default: Option<i64>,
) -> Result<i64, ApiError> {
    parse_optional_i64_param(params, key)?
        .or(default)
        .ok_or_else(|| {
            ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
        })
}

fn parse_optional_i64_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<i64>, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return Ok(None);
    };
    value
        .parse::<i64>()
        .map(Some)
        .map_err(|err| ApiError::bad_request(anyhow!("invalid integer for `{key}`: {err}")))
}

fn parse_optional_blocked_binding_constraint_source_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<BlockedBindingConstraintSource>, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return Ok(None);
    };
    BlockedBindingConstraintSource::from_query_param(&value)
        .ok_or_else(|| {
            ApiError::bad_request(anyhow!(
                "{key} must be one of: upstreamAccountBinding, encryptedSessionOwner"
            ))
        })
        .map(Some)
}

fn parse_optional_u8_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<u8>, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return Ok(None);
    };
    value
        .parse::<u8>()
        .map(Some)
        .map_err(|err| ApiError::bad_request(anyhow!("invalid integer for `{key}`: {err}")))
}

fn parse_bool_param(
    params: &BTreeMap<String, String>,
    key: &str,
    default: Option<bool>,
) -> Result<bool, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return default.ok_or_else(|| {
            ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
        });
    };
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ApiError::bad_request(anyhow!(
            "invalid boolean for `{key}`: {value}"
        ))),
    }
}

fn parse_conversation_subscription_scope(
    params: &BTreeMap<String, String>,
) -> Result<ConversationSubscriptionScope, ApiError> {
    let prompt_cache_key = parse_optional_text_param(params, "promptCacheKey");
    let sticky_key = parse_optional_text_param(params, "stickyKey");
    match (prompt_cache_key, sticky_key) {
        (Some(prompt_cache_key), None) => Ok(ConversationSubscriptionScope::PromptCacheKey(
            prompt_cache_key,
        )),
        (None, Some(sticky_key)) => {
            let upstream_account_id = parse_required_i64_param(params, "upstreamAccountId")?;
            if upstream_account_id <= 0 {
                return Err(ApiError::bad_request(anyhow!(
                    "upstreamAccountId must be positive for stickyKey subscription scope"
                )));
            }
            Ok(ConversationSubscriptionScope::StickyKey {
                sticky_key,
                upstream_account_id,
            })
        }
        (Some(_), Some(_)) => Err(ApiError::bad_request(anyhow!(
            "promptCacheKey and stickyKey are mutually exclusive subscription scope params"
        ))),
        (None, None) => Err(ApiError::bad_request(anyhow!(
            "promptCacheKey or stickyKey + upstreamAccountId is required for conversation subscription"
        ))),
    }
}

fn parse_optional_conversation_operation_info_type(
    params: &BTreeMap<String, String>,
) -> Result<Option<String>, ApiError> {
    let Some(info_type) = parse_optional_text_param(params, "infoType") else {
        return Ok(None);
    };
    match info_type.as_str() {
        PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING
        | PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_FORWARD_PROXY
        | PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE => Ok(Some(info_type)),
        _ => Err(ApiError::bad_request(anyhow!(
            "infoType must be one of: routing, forwardProxy, requestRewrite"
        ))),
    }
}

fn parse_prompt_cache_selection(
    params: &BTreeMap<String, String>,
) -> Result<PromptCacheConversationSelection, ApiError> {
    let limit = parse_optional_i64_param(params, "limit")?;
    let activity_hours = parse_optional_i64_param(params, "activityHours")?;
    let activity_minutes = parse_optional_i64_param(params, "activityMinutes")?;
    resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit,
        activity_hours,
        activity_minutes,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    })
}

fn parse_prompt_cache_detail_level(
    params: &BTreeMap<String, String>,
) -> Result<PromptCacheConversationDetailLevel, ApiError> {
    resolve_prompt_cache_conversation_detail_level(
        parse_optional_text_param(params, "detail").as_deref(),
    )
}

fn parse_sticky_selection(
    params: &BTreeMap<String, String>,
) -> Result<AccountStickyKeySelection, ApiError> {
    resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: parse_optional_i64_param(params, "limit")?,
        activity_hours: parse_optional_i64_param(params, "activityHours")?,
    })
    .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))
}

fn prompt_cache_selection_params(
    selection: PromptCacheConversationSelection,
) -> BTreeMap<String, String> {
    match selection {
        PromptCacheConversationSelection::Count(limit) => {
            BTreeMap::from([("limit".to_string(), limit.to_string())])
        }
        PromptCacheConversationSelection::ActivityWindowHours(hours) => {
            BTreeMap::from([("activityHours".to_string(), hours.to_string())])
        }
        PromptCacheConversationSelection::ActivityWindowMinutes(minutes) => {
            BTreeMap::from([("activityMinutes".to_string(), minutes.to_string())])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_mutation_sequence_gap_discards_partial_batch() {
        let mut last_sequence = 0;
        let first = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::AttemptChanged {
                invoke_id: "first".to_string(),
            },
        }];
        assert!(!runtime_mutation_batch_has_sequence_gap(
            &mut last_sequence,
            &first
        ));

        let gap = [SequencedRuntimeMutation {
            sequence: 3,
            mutation: RuntimeMutation::AttemptChanged {
                invoke_id: "after-gap".to_string(),
            },
        }];
        assert!(runtime_mutation_batch_has_sequence_gap(
            &mut last_sequence,
            &gap
        ));
        assert_eq!(last_sequence, 3);

        let recovered = [SequencedRuntimeMutation {
            sequence: 4,
            mutation: RuntimeMutation::AttemptChanged {
                invoke_id: "after-recovery".to_string(),
            },
        }];
        assert!(
            !runtime_mutation_batch_has_sequence_gap(&mut last_sequence, &recovered),
            "the first batch after recovery must advance from the discarded gap batch"
        );
    }

    #[tokio::test]
    async fn runtime_dependency_index_selects_only_the_matching_active_history_topic() {
        let hub = Arc::new(SubscriptionHub::new());
        let active_topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let inactive_topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("retained-key".to_string()),
        };
        let active_key = active_topic.cache_key().expect("active topic key");
        let inactive_key = inactive_topic.cache_key().expect("inactive topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                active_key.clone(),
                seeded_cached_topic(active_topic.clone(), &[], Utc::now()),
            );
            guard.topics.insert(
                inactive_key.clone(),
                seeded_cached_topic(inactive_topic, &[], Utc::now()),
            );
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&active_topic))
            .await
            .expect("register active history topic");
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new("invoke-1", "2026-08-09 12:00:00"),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("selected-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }];

        let guard = hub.state.lock().await;
        let work = SubscriptionHub::collect_runtime_topic_work(&guard, &mutations);
        let indexed = SubscriptionHub::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::HistoryPromptCacheKey("selected-key".to_string()),
        );
        assert_eq!(indexed, vec![active_key.clone()]);
        assert_eq!(work.len(), 1);
        assert_eq!(
            work[0].topic.cache_key().expect("work topic key"),
            active_key
        );
        assert_ne!(
            work[0].topic.cache_key().expect("work topic key"),
            inactive_key,
            "retained inactive topic must not become router work"
        );
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn releasing_last_owner_removes_dependency_index_and_marks_last_good_dirty() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("disconnect-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[], Utc::now()),
            );
        }
        let mut lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");
        let topic_keys = std::mem::take(&mut lease.topic_keys);
        let topic_names = std::mem::take(&mut lease.topic_names);
        hub.release_topic_subscribers(topic_keys, topic_names, lease.owns_dashboard_live)
            .await;
        drop(lease);

        let guard = hub.state.lock().await;
        assert!(!guard.active_topics.contains_key(&topic_key));
        assert!(
            SubscriptionHub::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::HistoryPromptCacheKey("disconnect-key".to_string()),
            )
            .is_empty()
        );
        assert!(
            guard
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.dirty),
            "the retained frame must rebuild before a future owner reconnects"
        );
    }

    #[tokio::test]
    async fn prompt_cache_dependency_index_omits_retained_inactive_windows() {
        let hub = Arc::new(SubscriptionHub::new());
        let active_topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let inactive_topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(10),
            detail_level: PromptCacheConversationDetailLevel::Compact,
            recent_invocation_limit: Some(5),
        };
        let active_key = active_topic
            .cache_key()
            .expect("active prompt cache topic key");
        let inactive_key = inactive_topic
            .cache_key()
            .expect("inactive prompt cache topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                active_key.clone(),
                seeded_cached_topic(active_topic.clone(), &[], Utc::now()),
            );
            guard.topics.insert(
                inactive_key.clone(),
                seeded_cached_topic(inactive_topic, &[], Utc::now()),
            );
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&active_topic))
            .await
            .expect("register active prompt cache topic");

        let guard = hub.state.lock().await;
        let projection_keys = SubscriptionHub::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::PromptCacheProjection,
        );
        let binding_keys = SubscriptionHub::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::PromptCacheWindow,
        );
        assert_eq!(projection_keys, vec![active_key.clone()]);
        assert_eq!(binding_keys, vec![active_key]);
        assert!(
            !projection_keys.contains(&inactive_key),
            "retained inactive prompt cache windows must not receive runtime deltas"
        );
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn inactive_prompt_cache_owner_skips_runtime_projection_materialization() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[7], Utc::now()),
            );
        }
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.invoke_id = "inactive-prompt-cache".to_string();
        record.prompt_cache_key = Some("cache-key".to_string());
        state.proxy_runtime_invocations.upsert(record.clone());
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeUpsert),
        }];

        hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
            .await;

        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("retained inactive prompt cache topic");
        assert_eq!(cached.cursor, 7);
        assert!(cached.prompt_cache_pending_records.is_empty());
        assert!(!cached.prompt_cache_refresh_scheduled);
        assert!(
            !guard
                .prompt_cache_prebaseline_records
                .contains_key(&topic_key),
            "an inactive owner must not receive deferred prompt cache work"
        );
    }

    #[tokio::test]
    async fn active_prompt_cache_projection_uses_typed_preview_without_full_record_clone() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        hub.state.lock().await.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[7], Utc::now()),
        );
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.invoke_id = "compact-prompt-cache-projection".to_string();
        record.prompt_cache_key = Some("cache-key".to_string());
        record.request_raw_path = Some("runtime-only-raw-path".repeat(256));
        state.proxy_runtime_invocations.upsert(record.clone());
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeUpsert),
        }];

        state
            .proxy_runtime_invocations
            .reset_full_record_clone_count();
        hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
            .await;

        assert_eq!(
            state.proxy_runtime_invocations.full_record_clone_count(),
            0,
            "active prompt cache projection must not clone ApiInvocation"
        );
        let guard = hub.state.lock().await;
        let delta = guard
            .topics
            .get(&topic_key)
            .and_then(|cached| cached.prompt_cache_pending_records.values().next())
            .expect("typed prompt cache delta");
        let preview = delta.preview.as_ref().expect("typed prompt cache preview");
        assert_eq!(preview.invoke_id, "compact-prompt-cache-projection");
        assert_eq!(preview.prompt_cache_key.as_deref(), Some("cache-key"));
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn runtime_gap_preserves_prompt_cache_last_good_and_defers_reconcile() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        let last_good = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        let last_good_frame = last_good.snapshot_frame.clone();
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(topic_key.clone(), last_good);
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");

        hub.mark_runtime_mutation_gap_and_recover(state.clone(), 4, "cursor_gap")
            .await;

        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("active prompt cache topic");
        assert!(cached.dirty);
        assert_eq!(cached.continuity_reset_cursor, Some(7));
        assert!(cached.prompt_cache_reconcile_required);
        assert_eq!(cached.prompt_cache_full_hydration_count, 0);
        assert!(Arc::ptr_eq(&cached.snapshot_frame, &last_good_frame));
        assert!(guard.runtime_topic_recovery_queue.is_empty());
        assert!(!guard.runtime_topic_recovery_running);
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn active_dirty_prompt_cache_last_good_degrades_aggregate_health() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.dirty = true;
        cached.prompt_cache_reconcile_required = true;
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");

        let stale = crate::load_runtime_pressure_health(state.as_ref()).await;
        assert_eq!(stale.state, "degraded");
        assert_eq!(
            stale.prompt_cache_projection.recovery_state,
            "failed_or_stale"
        );
        assert_eq!(stale.prompt_cache_projection.failed_or_stale_topic_count, 1);

        hub.state
            .lock()
            .await
            .topics
            .get_mut(&topic_key)
            .expect("active prompt cache topic")
            .prompt_cache_pressure_deferred = true;
        let deferred = crate::load_runtime_pressure_health(state.as_ref()).await;
        assert_eq!(deferred.state, "deferred");
        assert_eq!(
            deferred.prompt_cache_projection.recovery_state,
            "pressure_deferred"
        );
        assert_eq!(
            deferred
                .prompt_cache_projection
                .pressure_deferred_topic_count,
            1
        );
        drop(lease);
    }

    #[tokio::test]
    async fn prompt_cache_live_db_reads_degrade_aggregate_health() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let topic_key = topic.cache_key().expect("working conversation topic key");
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.prompt_cache_full_hydration_count = 2;
        cached.prompt_cache_bounded_key_hydration_count = 1;
        hub.state.lock().await.topics.insert(topic_key, cached);
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register working conversation owner");

        let health = crate::load_runtime_pressure_health(state.as_ref()).await;
        assert_eq!(health.prompt_cache_projection.live_path_db_read_count, 1);
        assert_eq!(health.prompt_cache_projection.recovery_state, "hot_db_read");
        assert_eq!(health.state, "degraded");
        drop(lease);
    }

    #[tokio::test]
    async fn prompt_cache_bounded_cold_recovery_defers_aggregate_health() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let topic_key = topic.cache_key().expect("working conversation topic key");
        let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        cached.prompt_cache_full_hydration_count = 1;
        cached.prompt_cache_bounded_key_hydration_count = 1;
        cached.prompt_cache_response_source = "database_bounded_key_hydrate";
        hub.state.lock().await.topics.insert(topic_key, cached);
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register working conversation owner");

        let health = crate::load_runtime_pressure_health(state.as_ref()).await;
        assert_eq!(health.prompt_cache_projection.live_path_db_read_count, 0);
        assert_eq!(
            health
                .prompt_cache_projection
                .bounded_cold_recovery_topic_count,
            1
        );
        assert_eq!(
            health.prompt_cache_projection.recovery_state,
            "bounded_cold_recovery"
        );
        assert_eq!(health.state, "deferred");
        drop(lease);
    }

    #[tokio::test]
    async fn repeated_runtime_gaps_dedupe_bounded_recovery_jobs() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[7], Utc::now()),
            );
            // Keep the worker parked so this test can observe the queued representation.
            guard.runtime_topic_recovery_running = true;
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");

        hub.mark_runtime_mutation_gap_and_recover(state.clone(), 2, "cursor_gap")
            .await;
        hub.mark_runtime_mutation_gap_and_recover(state.clone(), 3, "receiver_lagged")
            .await;

        let mut guard = hub.state.lock().await;
        assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
        assert_eq!(
            guard.runtime_topic_recovery_queue.front(),
            Some(&(topic_key.clone(), 1)),
            "the queued job retains its original generation and is re-enqueued once with the\n             newer generation after the worker observes the mismatch"
        );
        assert!(guard.runtime_topic_recovery_queued.contains(&topic_key));
        assert_eq!(
            guard
                .topics
                .get(&topic_key)
                .expect("active history topic")
                .runtime_topic_recovery_generation,
            2
        );
        guard.runtime_topic_recovery_running = false;
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn inactive_owner_skips_cold_prompt_cache_hydration() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };

        state.pool.close().await;

        assert!(
            hub.refresh_topic_if_active(state, topic, true)
                .await
                .expect("inactive guard must return before acquiring the closed database pool")
                .is_none()
        );
    }

    #[tokio::test]
    async fn active_owner_without_cached_topic_commits_guarded_refresh() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");

        let cached = hub
            .refresh_topic_if_active(state, topic, true)
            .await
            .expect("active owner can cold build")
            .expect("guarded refresh commits when no generation changed");

        assert_eq!(
            cached.topic.cache_key().expect("cached topic key"),
            topic_key
        );
        drop(lease);
    }

    #[tokio::test]
    async fn unrelated_topic_disconnect_does_not_block_active_refresh() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let active_topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("active-key".to_string()),
        };
        let released_topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("released-key".to_string()),
        };
        let active_key = active_topic.cache_key().expect("active topic key");
        let released_key = released_topic.cache_key().expect("released topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                active_key.clone(),
                seeded_cached_topic(active_topic.clone(), &[7], Utc::now()),
            );
            guard.topics.insert(
                released_key.clone(),
                seeded_cached_topic(released_topic.clone(), &[7], Utc::now()),
            );
        }
        let active_lease = hub
            .register_topic_subscribers(std::slice::from_ref(&active_topic))
            .await
            .expect("register active topic");
        let released_lease = hub
            .register_topic_subscribers(std::slice::from_ref(&released_topic))
            .await
            .expect("register released topic");
        hub.release_topic_subscribers(
            vec![released_key],
            vec![released_topic.name().to_string()],
            false,
        )
        .await;

        assert!(
            hub.refresh_topic_if_active(state, active_topic, true)
                .await
                .expect("active refresh")
                .is_some()
        );
        drop(released_lease);
        drop(active_lease);
    }

    #[tokio::test]
    async fn dirty_last_good_reconnect_skips_synchronous_cold_hydration() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        let mut last_good = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        last_good.dirty = true;
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(topic_key.clone(), last_good);
            // Keep the bounded recovery worker parked so this test can prove that preparing the
            // connection itself never acquires SQLite for a retained last-good frame.
            guard.runtime_topic_recovery_running = true;
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");
        state.pool.close().await;

        let prepared = hub
            .prepare_connection(state, vec![topic.descriptor()], Vec::new())
            .await
            .expect("reconnect must serve last-good without a database read");

        assert_eq!(prepared.initial.len(), 1);
        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .expect("retained history topic")
                .dirty
        );
        assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn runtime_recovery_retry_cooldown_defers_dirty_topic_requeue() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        {
            let mut guard = hub.state.lock().await;
            let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
            cached.dirty = true;
            guard.topics.insert(topic_key.clone(), cached);
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");

        assert_eq!(
            hub.defer_runtime_topic_recovery_retry(&topic).await,
            RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF
        );

        let mut guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .and_then(|cached| cached.runtime_topic_recovery_retry_at)
                .is_some_and(|retry_at| retry_at > Instant::now())
        );
        assert!(!SubscriptionHub::enqueue_runtime_topic_recovery_locked(
            &mut guard
        ));
        assert!(guard.runtime_topic_recovery_queue.is_empty());
        assert!(SubscriptionHub::next_runtime_topic_recovery_retry_delay_locked(&guard).is_some());
        guard
            .topics
            .get_mut(&topic_key)
            .expect("dirty history topic")
            .runtime_topic_recovery_retry_at = Some(Instant::now());
        assert!(SubscriptionHub::enqueue_runtime_topic_recovery_locked(
            &mut guard
        ));
        assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn newer_runtime_gap_clears_topic_recovery_cooldown() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        {
            let mut guard = hub.state.lock().await;
            let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
            cached.dirty = true;
            cached.runtime_topic_recovery_retry_at =
                Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
            guard.topics.insert(topic_key.clone(), cached);
            guard.runtime_topic_recovery_running = true;
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");

        hub.mark_runtime_mutation_gap_and_recover(state, 1, "cursor_gap")
            .await;

        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .expect("dirty history topic")
                .runtime_topic_recovery_retry_at
                .is_none()
        );
        assert_eq!(guard.runtime_topic_recovery_queue.len(), 1);
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn dirty_topics_do_not_bypass_bounded_gap_recovery() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::PromptCacheKey("selected-key".to_string()),
        };
        let topic_key = topic.cache_key().expect("history topic key");
        {
            let mut guard = hub.state.lock().await;
            let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
            cached.dirty = true;
            guard.topics.insert(topic_key, cached);
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register history owner");
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new("invoke-1", "2026-08-09 12:00:00"),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("selected-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }];

        let guard = hub.state.lock().await;
        assert!(SubscriptionHub::collect_runtime_topic_work(&guard, &mutations).is_empty());
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn dirty_prompt_cache_topic_retains_last_good_until_bounded_reconcile() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        let last_good = seeded_cached_topic(topic.clone(), &[7], Utc::now());
        let last_good_frame = last_good.snapshot_frame.clone();
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(topic_key.clone(), last_good);
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");
        hub.mark_runtime_mutation_gap_and_recover(state.clone(), 4, "cursor_gap")
            .await;

        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.invoke_id = "post-gap-prompt-cache".to_string();
        record.prompt_cache_key = Some("cache-key".to_string());
        state.proxy_runtime_invocations.upsert(record.clone());
        let mutations = [SequencedRuntimeMutation {
            sequence: 5,
            mutation: RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeUpsert),
        }];
        hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
            .await;

        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("active prompt cache topic");
        assert!(cached.dirty);
        assert!(cached.prompt_cache_reconcile_scheduled);
        assert!(!cached.prompt_cache_refresh_scheduled);
        assert!(cached.prompt_cache_pending_records.is_empty());
        assert_eq!(cached.cursor, 7);
        assert!(Arc::ptr_eq(&cached.snapshot_frame, &last_good_frame));
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn dirty_prompt_cache_reconcile_respects_baseline_cadence() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        {
            let mut guard = hub.state.lock().await;
            let mut cached = seeded_cached_topic(topic.clone(), &[7], Utc::now());
            cached.dirty = true;
            cached.prompt_cache_reconcile_required = true;
            cached.prompt_cache_baseline_at = Some(Instant::now());
            guard.topics.insert(topic_key, cached);
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");

        assert!(
            hub.prompt_cache_topic_reconcile_delay(&topic)
                .await
                .is_some_and(|delay| delay > Duration::from_secs(59))
        );
        drop(lease);
    }

    #[tokio::test]
    async fn prompt_cache_pressure_defer_wakes_on_eligibility_change() {
        let gate = DbPressureGate::new(1, Duration::from_millis(10));
        let permit = gate
            .try_begin_background("prompt_cache_topic_reconcile")
            .expect("occupy only background slot");
        let observed_eligibility = gate.eligibility_generation();
        let reason = gate
            .try_begin_background("prompt_cache_topic_reconcile")
            .expect_err("second background task is deferred");

        drop(permit);

        tokio::time::timeout(
            Duration::from_secs(1),
            wait_for_prompt_cache_reconcile_eligibility(&gate, observed_eligibility, reason),
        )
        .await
        .expect("eligible background slot wakes deferred prompt cache recovery");
    }

    #[tokio::test]
    async fn missing_runtime_identity_schedules_active_prompt_cache_reconcile() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[7], Utc::now()),
            );
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new("recovered", "2026-08-09 12:00:00"),
                kind: RuntimeMutationKind::Recovery,
                row_id: Some(1),
                is_terminal: true,
                prompt_cache_key: Some("cache-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }];

        hub.schedule_prompt_cache_topic_projection(state.clone(), &mutations)
            .await;

        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("active prompt cache topic");
        assert!(cached.dirty);
        assert!(cached.prompt_cache_reconcile_required);
        assert!(cached.prompt_cache_reconcile_scheduled);
        assert!(cached.prompt_cache_pending_records.is_empty());
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn prompt_cache_materialization_failure_marks_and_schedules_reconcile() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let topic_key = topic.cache_key().expect("prompt cache topic key");
        {
            let mut guard = hub.state.lock().await;
            guard.topics.insert(
                topic_key.clone(),
                seeded_cached_topic(topic.clone(), &[7], Utc::now()),
            );
        }
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register prompt cache owner");

        assert!(
            hub.mark_prompt_cache_topic_dirty_and_schedule_reconcile(&topic)
                .await
        );

        let guard = hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("active prompt cache topic");
        assert!(cached.dirty);
        assert!(cached.prompt_cache_reconcile_required);
        assert!(cached.prompt_cache_reconcile_scheduled);
        drop(guard);
        drop(lease);
    }

    fn summary_topic() -> SubscriptionTopic {
        SubscriptionTopic::SummaryCurrent {
            window: "current".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: Some(20),
            upstream_account_id: None,
        }
    }

    #[tokio::test]
    async fn first_dashboard_owner_starts_pending_runtime_projection_producer() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardActivityCurrent {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let selected_topics = vec![topic.clone(), summary_topic()];
        state.dashboard_network_speed_cache.record_request_bytes(
            "network-before-owner",
            "2026-08-04 12:00:00",
            Some(42),
            Some("api.openai.com"),
            128,
            Utc::now(),
        );
        schedule_dashboard_activity_live_snapshot(state.as_ref());
        assert!(
            state
                .proxy_runtime_invocations
                .pending_dashboard_deadline()
                .is_some()
        );
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(&selected_topics)
            .await
            .expect("register first dashboard owner");
        assert!(
            state
                .subscription_hub
                .has_active_dashboard_activity_live_topic()
                .await
        );
        assert_eq!(
            state
                .subscription_hub
                .dashboard_activity_live_subscriber_count()
                .await,
            1
        );
        let prepared = state
            .subscription_hub
            .prepare_connection(
                state.clone(),
                selected_topics
                    .iter()
                    .map(SubscriptionTopic::descriptor)
                    .collect(),
                Vec::new(),
            )
            .await
            .expect("prepare first dashboard owner connection");
        assert!(!prepared.initial.is_empty());
        ensure_dashboard_activity_live_snapshot_producer(state.as_ref());
    }

    fn dashboard_runtime_topology_descriptors() -> Vec<SubscriptionTopicDescriptor> {
        vec![
            SubscriptionTopicDescriptor {
                topic: "dashboard.activity.current".to_string(),
                params: BTreeMap::from([
                    ("range".to_string(), "today".to_string()),
                    (
                        "timeZone".to_string(),
                        SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                    ),
                    ("recentLimit".to_string(), "16".to_string()),
                    ("includeAccounts".to_string(), "true".to_string()),
                    ("includeRecent".to_string(), "true".to_string()),
                ]),
            },
            SubscriptionTopicDescriptor {
                topic: "stats.summary.current".to_string(),
                params: BTreeMap::from([
                    ("window".to_string(), "current".to_string()),
                    (
                        "timeZone".to_string(),
                        SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                    ),
                ]),
            },
            SubscriptionTopicDescriptor {
                topic: "dashboard.network-timeseries.window".to_string(),
                params: BTreeMap::from([
                    ("range".to_string(), "today".to_string()),
                    (
                        "timeZone".to_string(),
                        SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                    ),
                ]),
            },
            SubscriptionTopicDescriptor {
                topic: "dashboard.network-recent.current".to_string(),
                params: BTreeMap::new(),
            },
            SubscriptionTopicDescriptor {
                topic: "dashboard.working-conversations.current".to_string(),
                params: BTreeMap::from([
                    ("pageSize".to_string(), "20".to_string()),
                    ("recentInvocationLimit".to_string(), "16".to_string()),
                ]),
            },
            SubscriptionTopicDescriptor {
                topic: "stats.parallel-work.current".to_string(),
                params: BTreeMap::from([
                    ("range".to_string(), "1d".to_string()),
                    ("bucket".to_string(), "1m".to_string()),
                    (
                        "timeZone".to_string(),
                        SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                    ),
                ]),
            },
            SubscriptionTopicDescriptor {
                topic: "stats.timeseries.open-window".to_string(),
                params: BTreeMap::from([
                    ("range".to_string(), "1d".to_string()),
                    ("bucket".to_string(), "1m".to_string()),
                    (
                        "timeZone".to_string(),
                        SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                    ),
                ]),
            },
        ]
    }

    fn dashboard_runtime_topology_live_record(occurred_at: &str) -> ApiInvocation {
        ApiInvocation {
            id: 748_001,
            invoke_id: "dashboard-runtime-topology-live".to_string(),
            occurred_at: occurred_at.to_string(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: None,
            model: Some("gpt-5".to_string()),
            request_model: None,
            response_model: None,
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            total_tokens: None,
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: None,
            status: Some("running".to_string()),
            live_phase: Some("requesting".to_string()),
            error_message: None,
            downstream_status_code: None,
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            failure_class: None,
            is_actionable: None,
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            requester_ip: None,
            prompt_cache_key: None,
            sticky_key: None,
            route_mode: None,
            upstream_account_id: Some(42),
            upstream_account_name: Some("Topology Account".to_string()),
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            pool_attempt_count: None,
            pool_distinct_account_count: None,
            pool_attempt_terminal_reason: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            proxy_weight_delta: None,
            cost_estimated: None,
            price_version: None,
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: occurred_at.to_string(),
        }
    }

    fn dashboard_runtime_topology_stream_query(attempt: u64) -> SubscriptionStreamQuery {
        SubscriptionStreamQuery {
            topics: Some(
                serde_json::to_string(&dashboard_runtime_topology_descriptors())
                    .expect("serialize dashboard topology topics"),
            ),
            resume: None,
            attempt: Some(attempt),
            reason: Some(DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON.to_string()),
        }
    }

    async fn collect_dashboard_runtime_topology_sse_events(
        response: Response,
        required_working_keys: &[&str],
    ) -> BTreeMap<String, Value> {
        let expected = [
            "dashboard.activity.current",
            "stats.summary.current",
            "dashboard.network-timeseries.window",
            "dashboard.network-recent.current",
            "dashboard.working-conversations.current",
            "stats.parallel-work.current",
            "stats.timeseries.open-window",
        ];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut stream = response.into_body().into_data_stream();
        let mut buffered = Vec::new();
        let mut events: BTreeMap<String, Value> = BTreeMap::new();
        loop {
            let working_keys_ready = events
                .get("dashboard.working-conversations.current")
                .and_then(|envelope| envelope.pointer("/payload/conversations"))
                .and_then(Value::as_array)
                .map(|conversations| {
                    required_working_keys.iter().all(|required_key| {
                        conversations.iter().any(|conversation| {
                            conversation["promptCacheKey"].as_str() == Some(required_key)
                        })
                    })
                })
                .unwrap_or(required_working_keys.is_empty());
            if events.len() == expected.len() && working_keys_ready {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "dashboard topology SSE live event timeout; received={:?}",
                events.keys()
            );
            let chunk = tokio::time::timeout(remaining, stream.next())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "dashboard topology SSE live event timeout; received={:?}",
                        events.keys()
                    )
                })
                .expect("dashboard topology SSE stream closed")
                .expect("dashboard topology SSE stream chunk");
            buffered.extend_from_slice(&chunk);
            while let Some(event_end) = buffered.windows(2).position(|window| window == b"\n\n") {
                let event = buffered.drain(..event_end + 2).collect::<Vec<_>>();
                let Some(payload) = event
                    .strip_prefix(b"data: ")
                    .and_then(|payload| payload.strip_suffix(b"\n\n"))
                else {
                    continue;
                };
                let envelope: Value =
                    serde_json::from_slice(payload).expect("Dashboard SSE envelope");
                let Some(topic) = envelope
                    .pointer("/topic/topic")
                    .and_then(Value::as_str)
                    .filter(|topic| expected.contains(topic))
                else {
                    continue;
                };
                if envelope.get("type").and_then(Value::as_str) == Some("live") {
                    events.insert(topic.to_string(), envelope);
                }
            }
        }
        events
    }

    #[tokio::test]
    async fn dashboard_runtime_topology_materializes_shared_frames_without_business_payloads() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        state
            .proxy_runtime_invocations
            .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
            .expect("bind dashboard network cache");
        state
            .proxy_runtime_invocations
            .capture_memory_snapshot()
            .expect("establish in-memory dashboard projection");
        if let Some(window) = state
            .proxy_runtime_invocations
            .pending_dashboard_publish_window()
        {
            let consumed = state
                .proxy_runtime_invocations
                .begin_dashboard_publish_window(window)
                .expect("consume initial dashboard projection window");
            state
                .proxy_runtime_invocations
                .complete_dashboard_publish_window(consumed);
        }

        let descriptors = dashboard_runtime_topology_descriptors();
        let topics = descriptors
            .iter()
            .map(SubscriptionTopic::from_descriptor)
            .collect::<Result<Vec<_>, _>>()
            .expect("dashboard topology topics");
        let first_response = topic_sse_stream(
            State(state.clone()),
            Query(dashboard_runtime_topology_stream_query(1)),
        )
        .await
        .expect("open first full Dashboard SSE topology");
        let second_response = topic_sse_stream(
            State(state.clone()),
            Query(dashboard_runtime_topology_stream_query(2)),
        )
        .await
        .expect("open second full Dashboard SSE topology");
        for topic in &topics {
            assert_eq!(
                state
                    .subscription_hub
                    .active_topic_subscriber_count(topic.name())
                    .await,
                2,
                "SSE entrypoint should retain both Dashboard owners for {}",
                topic.name(),
            );
        }
        for topic in &topics {
            let expected_typed_materializer = matches!(
                topic.name(),
                "dashboard.activity.current"
                    | "stats.summary.current"
                    | "dashboard.network-timeseries.window"
                    | "dashboard.network-recent.current"
                    | "stats.timeseries.open-window"
                    | "stats.parallel-work.current"
                    | "dashboard.working-conversations.current"
            );
            assert_eq!(
                state
                    .subscription_hub
                    .dashboard_topic_uses_typed_materializer(topic)
                    .await,
                expected_typed_materializer,
                "typed materializer seam must report the current implementation for {}",
                topic.name(),
            );
        }
        {
            let guard = state.subscription_hub.state.lock().await;
            let working_topic_key = topics
                .iter()
                .find(|topic| topic.name() == "dashboard.working-conversations.current")
                .expect("working conversations topic")
                .cache_key()
                .expect("working conversations topic key");
            assert_eq!(
                guard
                    .server_push_subscribers
                    .get(&working_topic_key)
                    .copied(),
                Some(2),
                "working conversations owns its bounded expiry and reconcile cadence",
            );
            assert!(
                guard.server_push_tasks.contains(&working_topic_key),
                "working conversations must retain its bounded expiry task",
            );
        }
        spawn_subscription_broadcast_listener(state.clone());

        let process_started_epoch_second = state
            .dashboard_network_speed_cache
            .process_started_at_utc()
            .timestamp();
        while Utc::now().timestamp() <= process_started_epoch_second.saturating_add(1) {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        state
            .proxy_runtime_invocations
            .reset_dashboard_topology_counters();
        state.subscription_hub.reset_dashboard_topology_counters();
        state
            .subscription_hub
            .reset_dashboard_topology_sse_frame_observations()
            .await;
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        state
            .proxy_runtime_invocations
            .upsert(dashboard_runtime_topology_live_record(&occurred_at));
        let current_capture = state
            .proxy_runtime_invocations
            .capture_memory_snapshot()
            .expect("capture deterministic Dashboard current slice");
        assert!(current_capture.changed);
        state
            .subscription_hub
            .handle_internal_broadcast(
                state.clone(),
                BroadcastPayload::DashboardCurrentSlice {
                    slice: Box::new(DashboardCurrentProjectionSlice::from(
                        &current_capture.snapshot,
                    )),
                },
            )
            .await;
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 748_003;
        terminal.invoke_id = "dashboard-runtime-topology-terminal".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        terminal.t_total_ms = Some(3_450.0);
        terminal.t_req_read_ms = Some(10.0);
        terminal.t_req_parse_ms = Some(12.0);
        terminal.t_upstream_connect_ms = Some(18.0);
        terminal.t_upstream_ttfb_ms = Some(508.0);
        terminal.first_token_ms = Some(650.0);
        terminal.t_upstream_stream_ms = Some(2_800.0);
        let terminal_delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
            .await
            .terminal_delta
            .expect("accept terminal projection delta");
        state
            .proxy_runtime_invocations
            .record_dashboard_terminal_delta(terminal_delta);
        let terminal_capture = state
            .proxy_runtime_invocations
            .capture_terminal_slice()
            .expect("capture one terminal projection slice");
        state
            .subscription_hub
            .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
                revision: terminal_capture.revision,
                deltas: terminal_capture.deltas,
            })
            .await;
        state.dashboard_network_speed_cache.record_request_bytes(
            "dashboard-runtime-topology-network",
            &occurred_at,
            Some(42),
            Some("api.openai.com"),
            4096,
            Utc::now() - ChronoDuration::seconds(1),
        );
        let network_capture = state
            .proxy_runtime_invocations
            .capture_network_slice()
            .expect("capture deterministic Dashboard network slice");
        assert!(network_capture.changed);
        state
            .subscription_hub
            .handle_internal_broadcast(
                state.clone(),
                BroadcastPayload::DashboardNetworkSlice {
                    slice: Box::new(network_capture.slice),
                },
            )
            .await;
        let mut fallback = terminal.clone();
        fallback.id = 748_004;
        fallback.invoke_id = "dashboard-runtime-topology-fallback".to_string();
        // Parallel-work only reports completed buckets, so place the live mutation in the
        // previous minute while still applying it after both subscriptions are established.
        fallback.occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(1))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        fallback.created_at = fallback.occurred_at.clone();
        fallback.prompt_cache_key = Some("dashboard-runtime-topology-fallback".to_string());
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(fallback.id)
        .bind(fallback.invoke_id.as_str())
        .bind(fallback.occurred_at.as_str())
        .bind(fallback.source.as_str())
        .bind("success")
        .bind(42_i64)
        .bind(0.25_f64)
        .bind(
            json!({
                "promptCacheKey": fallback.prompt_cache_key.as_deref(),
                "upstreamAccountId": fallback.upstream_account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("persist generic fallback benchmark invocation");
        let mut second_parallel_work = fallback.clone();
        second_parallel_work.id = 748_005;
        second_parallel_work.invoke_id =
            "dashboard-runtime-topology-second-parallel-work".to_string();
        second_parallel_work.prompt_cache_key =
            Some("dashboard-runtime-topology-second".to_string());
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(second_parallel_work.id)
        .bind(second_parallel_work.invoke_id.as_str())
        .bind(second_parallel_work.occurred_at.as_str())
        .bind(second_parallel_work.source.as_str())
        .bind("success")
        .bind(42_i64)
        .bind(0.25_f64)
        .bind(
            json!({
                "promptCacheKey": second_parallel_work.prompt_cache_key.as_deref(),
                "upstreamAccountId": second_parallel_work.upstream_account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("persist second parallel-work benchmark invocation");
        let mut parallel_work_mutations = Vec::with_capacity(10_000);
        parallel_work_mutations.push(SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::invocation(&fallback, RuntimeMutationKind::RuntimeUpsert),
        });
        parallel_work_mutations.push(SequencedRuntimeMutation {
            sequence: 2,
            mutation: RuntimeMutation::invocation(
                &second_parallel_work,
                RuntimeMutationKind::RuntimeUpsert,
            ),
        });
        parallel_work_mutations.extend((3..=10_000).map(|sequence| SequencedRuntimeMutation {
            sequence,
            mutation: RuntimeMutation::invocation(&fallback, RuntimeMutationKind::RuntimeUpsert),
        }));
        state.proxy_runtime_invocations.upsert(fallback.clone());
        state
            .subscription_hub
            .handle_runtime_mutation_batch(state.clone(), parallel_work_mutations)
            .await;
        let parallel_topic = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        };
        let exact_parallel = load_parallel_work_stats_response(
            &state,
            ParallelWorkStatsQuery {
                range: "1d".to_string(),
                bucket: Some("1m".to_string()),
                time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
                upstream_account_id: None,
            },
        )
        .await
        .expect("build exact parallel-work response");
        let fallback_occurred_at =
            parse_to_utc_datetime(&fallback.occurred_at).expect("parse fallback timestamp");
        let fallback_bucket_start = format_utc_iso(
            Utc.timestamp_opt(fallback_occurred_at.timestamp().div_euclid(60) * 60, 0)
                .single()
                .expect("construct fallback minute"),
        );
        let exact_point = exact_parallel
            .current
            .points
            .iter()
            .find(|point| point.bucket_start == fallback_bucket_start)
            .expect("exact fallback point");
        let projection_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let projected_parallel = loop {
            let projected_parallel = {
                let guard = state.subscription_hub.state.lock().await;
                guard.topics[&parallel_topic.cache_key().expect("parallel-work topic key")]
                    .snapshot_frame
                    .payload_value()
            };
            let materialized_point =
                projected_parallel["current"]["points"]
                    .as_array()
                    .and_then(|points| {
                        points.iter().find(|point| {
                            point["bucketStart"].as_str() == Some(fallback_bucket_start.as_str())
                        })
                    });
            if materialized_point
                .is_some_and(|point| point["parallelCount"] == json!(exact_point.parallel_count))
            {
                break projected_parallel;
            }
            let remaining =
                projection_deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "parallel-work projection did not materialize the expected bucket"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let projected_point = projected_parallel["current"]["points"]
            .as_array()
            .and_then(|points| {
                points.iter().find(|point| {
                    point["bucketStart"].as_str() == Some(fallback_bucket_start.as_str())
                })
            })
            .expect("projected fallback point");
        assert_eq!(
            projected_point["parallelCount"],
            json!(exact_point.parallel_count),
            "runtime projection must preserve the exact distinct-key bucket count"
        );
        let projected_conversation = projected_parallel["current"]["conversations"]
            .as_array()
            .and_then(|conversations| {
                conversations.iter().find(|conversation| {
                    conversation["conversationId"].as_str() == fallback.prompt_cache_key.as_deref()
                })
            })
            .expect("projected fallback conversation");
        let exact_conversation = exact_parallel
            .current
            .conversations
            .iter()
            .find(|conversation| {
                Some(conversation.conversation_id.as_str()) == fallback.prompt_cache_key.as_deref()
            })
            .expect("exact fallback conversation");
        assert_eq!(
            projected_conversation,
            &serde_json::to_value(exact_conversation).expect("serialize exact conversation"),
            "runtime projection must preserve the exact conversation span"
        );
        let delivery_before_reconnect = state.subscription_hub.dashboard_topology_counters();
        for topic in [
            delivery_before_reconnect.activity,
            delivery_before_reconnect.summary,
            delivery_before_reconnect.network_timeseries,
            delivery_before_reconnect.network_recent,
            delivery_before_reconnect.working_conversations,
            delivery_before_reconnect.parallel_work,
            delivery_before_reconnect.timeseries,
        ] {
            assert_eq!(
                topic.active_subscriber_count, 2,
                "the live Dashboard topology must retain two subscribers before reconnect"
            );
            assert_eq!(
                topic.reconnect_churn_count, 0,
                "runtime mutations must not create reconnect churn"
            );
        }
        let resume = {
            let guard = state.subscription_hub.state.lock().await;
            topics
                .iter()
                .map(|topic| {
                    let topic_key = topic.cache_key().expect("Dashboard topic key");
                    let cached = guard
                        .topics
                        .get(&topic_key)
                        .expect("cached Dashboard topic");
                    SubscriptionResumeCursor {
                        topic_key,
                        cursor: cached.cursor,
                        schema_epoch: cached.schema_epoch.clone(),
                    }
                })
                .collect::<Vec<_>>()
        };
        let resumed = state
            .subscription_hub
            .prepare_connection(state.clone(), descriptors, resume)
            .await
            .expect("reconnect Dashboard topology with cursors");
        assert!(resumed.initial.is_empty());
        assert!(
            resumed
                .outcomes
                .iter()
                .all(|outcome| outcome.disposition == TopicInitDisposition::ResumeCaughtUp),
            "same-cursor reconnect must not rebuild the Dashboard bundle"
        );
        let (first_frames, second_frames) = tokio::join!(
            collect_dashboard_runtime_topology_sse_events(
                first_response,
                &[
                    "dashboard-runtime-topology-fallback",
                    "dashboard-runtime-topology-second",
                ],
            ),
            collect_dashboard_runtime_topology_sse_events(
                second_response,
                &[
                    "dashboard-runtime-topology-fallback",
                    "dashboard-runtime-topology-second",
                ],
            ),
        );
        state.shutdown.cancel();
        assert_eq!(
            first_frames, second_frames,
            "both SSE owners should receive the same serialized live frames",
        );
        let working_payload = first_frames
            .get("dashboard.working-conversations.current")
            .and_then(|envelope| envelope.get("payload"))
            .expect("working conversations live payload");
        assert_eq!(working_payload["totalMatched"], json!(2));
        let working_keys = working_payload["conversations"]
            .as_array()
            .expect("working conversations array")
            .iter()
            .filter_map(|conversation| conversation["promptCacheKey"].as_str())
            .collect::<HashSet<_>>();
        assert!(working_keys.contains("dashboard-runtime-topology-fallback"));
        assert!(
            working_keys.contains("dashboard-runtime-topology-second"),
            "a durable terminal missing from runtime store must hydrate its selected key"
        );
        let first_observations = state
            .subscription_hub
            .dashboard_topology_sse_frame_observations(1)
            .await;
        let second_observations = state
            .subscription_hub
            .dashboard_topology_sse_frame_observations(2)
            .await;
        for topic in [
            "dashboard.activity.current",
            "stats.summary.current",
            "dashboard.network-timeseries.window",
            "dashboard.network-recent.current",
            "dashboard.working-conversations.current",
            "stats.parallel-work.current",
            "stats.timeseries.open-window",
        ] {
            let first_owner_frames = first_observations
                .get(topic)
                .expect("first owner should observe each Dashboard live frame");
            let second_owner_frames = second_observations
                .get(topic)
                .expect("second owner should observe each Dashboard live frame");
            assert!(
                first_owner_frames
                    .iter()
                    .any(|first_frame| second_owner_frames
                        .iter()
                        .any(|second_frame| Arc::ptr_eq(first_frame, second_frame))),
                "topic {topic} should reuse one serialized frame across SSE owners",
            );
        }
        for (topic, terminal_total) in [
            ("dashboard.activity.current", json!(1)),
            ("stats.summary.current", json!(1)),
        ] {
            let first_owner_frames = first_observations
                .get(topic)
                .expect("first owner should observe terminal Dashboard frame");
            let second_owner_frames = second_observations
                .get(topic)
                .expect("second owner should observe terminal Dashboard frame");
            let terminal_frame = first_owner_frames
                .iter()
                .find(|frame| {
                    let payload = frame.payload_value();
                    let total = if topic == "dashboard.activity.current" {
                        &payload["summary"]["stats"]["totalCount"]
                    } else {
                        &payload["totalCount"]
                    };
                    total == &terminal_total
                })
                .expect("first owner should observe the terminal slice frame");
            assert!(
                second_owner_frames
                    .iter()
                    .any(|frame| Arc::ptr_eq(terminal_frame, frame)),
                "terminal topic {topic} must reuse one frame across two SSE owners",
            );
        }

        let projection = state
            .proxy_runtime_invocations
            .dashboard_topology_counters();
        assert_eq!(projection.current.build_count, 1);
        assert_eq!(projection.current.revision_count, 1);
        assert!(projection.current.cadence_miss_count <= 1);
        assert_eq!(projection.network.build_count, 1);
        assert_eq!(projection.network.revision_count, 1);
        assert_eq!(projection.network.cadence_miss_count, 0);
        assert_eq!(projection.terminal.build_count, 1);
        assert_eq!(projection.terminal.revision_count, 1);
        assert_eq!(projection.terminal.cadence_miss_count, 0);
        assert_eq!(
            state
                .proxy_runtime_invocations
                .health_snapshot(2)
                .live_path_db_read_count,
            0,
            "active Dashboard producer must remain on the in-memory live path",
        );

        let delivery = state.subscription_hub.dashboard_topology_counters();
        for topic in [
            delivery.activity,
            delivery.summary,
            delivery.network_timeseries,
            delivery.network_recent,
            delivery.working_conversations,
            delivery.parallel_work,
            delivery.timeseries,
        ] {
            assert_eq!(
                topic.business_payload_count, 0,
                "new Dashboard delivery must not fan out business payloads"
            );
            assert_eq!(
                topic.json_overlay_count, 0,
                "new Dashboard delivery must not mutate generic JSON payloads"
            );
            assert!(
                topic.materialization_count >= 1,
                "each active Dashboard topic should materialize an immutable frame"
            );
            assert_eq!(topic.builder_count, topic.materialization_count);
            assert!(topic.frame_bytes_count > 0);
            assert_eq!(topic.cursor_advanced, topic.serialization_count);
            assert!(topic.frame_reused > 0);
            assert_eq!(topic.lagged_count, 0);
            assert_eq!(topic.skipped_count, 0);
            assert_eq!(
                topic.reconnect_churn_count, 0,
                "a cursor-continuous resume is not reconnect churn"
            );
        }
        for topic in [delivery.activity, delivery.summary] {
            assert_eq!(
                topic.serialization_count, topic.materialization_count,
                "each materialized topic revision should serialize exactly once"
            );
            assert_eq!(
                topic.payload_clone_count, 0,
                "activity and summary must mutate typed bases without complete payload clones"
            );
        }
        for topic in [delivery.network_timeseries, delivery.network_recent] {
            assert_eq!(
                topic.serialization_count, topic.materialization_count,
                "each materialized network topic revision should serialize exactly once"
            );
            assert_eq!(
                topic.payload_clone_count, 0,
                "network topic materialization must borrow its typed base"
            );
        }
        assert_eq!(
            delivery.timeseries.serialization_count, delivery.timeseries.materialization_count,
            "each materialized timeseries revision should serialize exactly once"
        );
        assert_eq!(
            delivery.timeseries.payload_clone_count, 0,
            "timeseries topic materialization must retain typed aggregates"
        );
        for topic in [
            delivery.activity,
            delivery.summary,
            delivery.network_timeseries,
            delivery.network_recent,
            delivery.timeseries,
            delivery.working_conversations,
            delivery.parallel_work,
        ] {
            assert_eq!(topic.generic_fallback_build_count, 0);
            assert_eq!(topic.live_path_db_read_count, 0);
        }
        assert!(
            !state
                .subscription_hub
                .dashboard_delivery_has_degraded_signal(),
            "typed Dashboard topics must remain healthy on the in-memory live path"
        );
    }

    #[test]
    fn parallel_work_projection_extends_rolling_window_with_live_bucket() {
        let current_bucket = Utc::now().timestamp().div_euclid(60) * 60;
        let initial_start = Utc
            .timestamp_opt(current_bucket - 120, 0)
            .single()
            .expect("construct initial point start");
        let initial_end = Utc
            .timestamp_opt(current_bucket - 60, 0)
            .single()
            .expect("construct initial point end");
        let live_end = Utc
            .timestamp_opt(current_bucket + 60, 0)
            .single()
            .expect("construct live point end");
        let mut window = ParallelWorkWindowResponse {
            range_start: format_utc_iso(initial_start),
            range_end: format_utc_iso(initial_end),
            bucket_seconds: 60,
            complete_bucket_count: 1,
            active_bucket_count: 0,
            active_minute_count: Some(0),
            min_count: Some(0),
            max_count: Some(0),
            avg_count: None,
            effective_time_zone: chrono_tz::UTC.to_string(),
            time_zone_fallback: false,
            points: vec![ParallelWorkPoint {
                bucket_start: format_utc_iso(initial_start),
                bucket_end: format_utc_iso(initial_end),
                parallel_count: 0,
            }],
            conversations: Vec::new(),
        };
        let bucket_keys =
            BTreeMap::from([(current_bucket, HashSet::from(["live-key".to_string()]))]);

        assert!(refresh_parallel_work_points(
            &mut window,
            current_bucket,
            chrono_tz::UTC,
            &bucket_keys,
        ));
        assert_eq!(window.range_end, format_utc_iso(live_end));
        assert_eq!(window.complete_bucket_count, 3);
        assert_eq!(
            window.points.last().map(|point| point.parallel_count),
            Some(1)
        );
    }

    fn parallel_work_materializer_state(
        range: &str,
        range_start: DateTime<Utc>,
        baseline_row_id: i64,
        upstream_account_id: Option<i64>,
    ) -> DashboardParallelWorkMaterializerState {
        let range_end = range_start + ChronoDuration::minutes(1);
        let window = ParallelWorkWindowResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            bucket_seconds: 60,
            complete_bucket_count: 1,
            active_bucket_count: 0,
            active_minute_count: Some(0),
            min_count: Some(0),
            max_count: Some(0),
            avg_count: None,
            effective_time_zone: chrono_tz::UTC.to_string(),
            time_zone_fallback: false,
            points: vec![ParallelWorkPoint {
                bucket_start: format_utc_iso(range_start),
                bucket_end: format_utc_iso(range_end),
                parallel_count: 0,
            }],
            conversations: vec![ParallelWorkConversation {
                conversation_id: "baseline-key".to_string(),
                start: format_utc_iso(range_start),
                end: format_utc_iso(range_end),
                request_count: 1,
            }],
        };
        let response = ParallelWorkStatsResponse {
            current: window.clone(),
            minute7d: window.clone(),
            hour30d: window.clone(),
            day_all: window,
        };
        let range_start_epoch = range_start.timestamp();
        DashboardParallelWorkMaterializerState {
            baseline_response: response.clone(),
            response,
            baseline_bucket_keys: BTreeMap::new(),
            bucket_keys: BTreeMap::new(),
            baseline_minute_keys: BTreeMap::new(),
            minute_keys: BTreeMap::new(),
            baseline_active_minute_stats: ParallelWorkActiveMinuteStats::default(),
            active_minute_stats: ParallelWorkActiveMinuteStats::default(),
            baseline_complete_minute_start_epoch: if range_start_epoch.rem_euclid(60) == 0 {
                range_start_epoch
            } else {
                range_start_epoch.div_euclid(60) * 60 + 60
            },
            baseline_complete_minute_end_epoch: range_end.timestamp().div_euclid(60) * 60,
            baseline_row_id,
            range: range.to_string(),
            reporting_tz: chrono_tz::UTC,
            upstream_account_id,
            conversations_enabled: true,
            baseline_identities: HashSet::new(),
            applied_identities: HashSet::new(),
            runtime_mutations: BTreeMap::new(),
            revision: 0,
        }
    }

    #[test]
    fn parallel_work_projection_excludes_incomplete_current_minute_from_average() {
        let now = Utc::now();
        let current_minute_start = now.timestamp().div_euclid(60) * 60;
        let occurred_at = Utc
            .timestamp_opt(current_minute_start + 1, 0)
            .single()
            .expect("construct current incomplete minute");
        let mut state = parallel_work_materializer_state(
            "1d",
            occurred_at - ChronoDuration::minutes(2),
            0,
            None,
        );
        let active_minute_stats = ParallelWorkActiveMinuteStats {
            active_minute_count: Some(4),
            parallel_count_sum: 12,
        };
        state.baseline_active_minute_stats = active_minute_stats;
        state.active_minute_stats = active_minute_stats;
        for window in [
            &mut state.response.current,
            &mut state.response.minute7d,
            &mut state.response.hour30d,
            &mut state.response.day_all,
        ] {
            window.active_minute_count = Some(4);
            window.avg_count = Some(3.0);
        }

        assert!(state.apply_runtime_overlay_at(
            &RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new(
                    "current-minute-invoke",
                    format_utc_iso(occurred_at),
                ),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("current-minute-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            },
            now,
        ));
        assert_eq!(state.active_minute_stats, active_minute_stats);
        assert_eq!(state.response.current.active_minute_count, Some(4));
        assert_eq!(state.response.current.avg_count, Some(3.0));
    }

    #[test]
    fn parallel_work_projection_promotes_closed_runtime_minutes_on_next_overlay() {
        let minute_start = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("construct minute start");
        let mut state = parallel_work_materializer_state(
            "1d",
            minute_start - ChronoDuration::minutes(2),
            0,
            None,
        );
        let active_minute_stats = ParallelWorkActiveMinuteStats {
            active_minute_count: Some(4),
            parallel_count_sum: 12,
        };
        state.baseline_active_minute_stats = active_minute_stats;
        state.active_minute_stats = active_minute_stats;
        for window in [
            &mut state.response.current,
            &mut state.response.minute7d,
            &mut state.response.hour30d,
            &mut state.response.day_all,
        ] {
            window.active_minute_count = Some(4);
            window.avg_count = Some(3.0);
        }

        let first = RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "completed-minute-invoke",
                format_utc_iso(minute_start + ChronoDuration::seconds(1)),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("completed-minute-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        assert!(state.apply_runtime_overlay_at(&first, minute_start + ChronoDuration::seconds(30)));
        assert_eq!(state.active_minute_stats, active_minute_stats);

        let next_minute = RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "next-minute-invoke",
                format_utc_iso(minute_start + ChronoDuration::seconds(61)),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("next-minute-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        assert!(
            state
                .apply_runtime_overlay_at(&next_minute, minute_start + ChronoDuration::seconds(90))
        );
        assert_eq!(
            state.active_minute_stats,
            ParallelWorkActiveMinuteStats {
                active_minute_count: Some(5),
                parallel_count_sum: 13,
            }
        );
        assert_eq!(state.response.current.avg_count, Some(2.6));
    }

    #[test]
    fn parallel_work_projection_promotes_persisted_current_minute_after_boundary() {
        let minute_start = Utc
            .timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("construct minute start");
        let mut state = parallel_work_materializer_state(
            "1d",
            minute_start - ChronoDuration::minutes(2),
            0,
            None,
        );
        let active_minute_stats = ParallelWorkActiveMinuteStats {
            active_minute_count: Some(4),
            parallel_count_sum: 12,
        };
        state.baseline_active_minute_stats = active_minute_stats;
        state.active_minute_stats = active_minute_stats;
        state.baseline_complete_minute_end_epoch = minute_start.timestamp();
        state.baseline_minute_keys.insert(
            minute_start.timestamp(),
            HashSet::from(["persisted-current-minute-key".to_string()]),
        );
        state.minute_keys = state.baseline_minute_keys.clone();
        for window in [
            &mut state.response.current,
            &mut state.response.minute7d,
            &mut state.response.hour30d,
            &mut state.response.day_all,
        ] {
            window.active_minute_count = Some(4);
            window.avg_count = Some(3.0);
        }

        let next_minute = RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "next-minute-invoke",
                format_utc_iso(minute_start + ChronoDuration::seconds(61)),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("next-minute-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        assert!(
            state
                .apply_runtime_overlay_at(&next_minute, minute_start + ChronoDuration::seconds(90))
        );
        assert_eq!(
            state.active_minute_stats,
            ParallelWorkActiveMinuteStats {
                active_minute_count: Some(5),
                parallel_count_sum: 13,
            }
        );
        assert_eq!(state.response.current.avg_count, Some(2.6));
    }

    #[test]
    fn parallel_work_projection_skips_rows_already_in_its_cold_baseline() {
        let occurred_at = Utc::now() - ChronoDuration::seconds(30);
        let mut state = parallel_work_materializer_state("1d", occurred_at, 42, None);
        let outcome = state.apply_runtime_mutation(&RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "baseline-invoke",
                format_utc_iso(occurred_at),
            ),
            kind: RuntimeMutationKind::TerminalCommitted,
            row_id: Some(42),
            is_terminal: true,
            prompt_cache_key: Some("baseline-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        });

        assert!(!outcome.changed);
        assert_eq!(state.response.current.conversations[0].request_count, 1);
    }

    #[test]
    fn parallel_work_projection_requires_typed_reconcile_for_unknown_account_fallback() {
        let occurred_at = Utc::now() - ChronoDuration::seconds(30);
        let mut state = parallel_work_materializer_state("1d", occurred_at, 0, Some(77));
        let outcome = state.apply_runtime_mutation(&RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "fallback-invoke",
                format_utc_iso(occurred_at),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: Some(43),
            is_terminal: false,
            prompt_cache_key: Some("account-fallback-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        });

        assert!(!outcome.changed);
        assert!(outcome.needs_account_reconcile);
    }

    #[test]
    fn parallel_work_projection_rebases_moving_ranges() {
        let state = parallel_work_materializer_state(
            "1d",
            Utc::now() - ChronoDuration::minutes(2),
            0,
            None,
        );

        assert!(state.requires_rolling_rebase());
    }

    #[test]
    fn parallel_work_projection_removes_runtime_only_work_immediately() {
        let occurred_at = Utc::now();
        let mut state = parallel_work_materializer_state(
            "1d",
            occurred_at - ChronoDuration::minutes(2),
            0,
            None,
        );
        let identity =
            RuntimeInvocationIdentity::new("runtime-only-invoke", format_utc_iso(occurred_at));
        let upsert = RuntimeInvocationMutation {
            identity: identity.clone(),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("runtime-only-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        assert!(state.apply_runtime_mutation(&upsert).changed);
        assert!(
            state
                .response
                .current
                .conversations
                .iter()
                .any(|conversation| { conversation.conversation_id == "runtime-only-key" })
        );

        let removed = state.apply_runtime_mutation(&RuntimeInvocationMutation {
            identity,
            kind: RuntimeMutationKind::RuntimeRemoved,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: None,
            sticky_key: None,
            upstream_account_id: None,
        });
        assert!(removed.changed);
        assert!(
            !state
                .response
                .current
                .conversations
                .iter()
                .any(|conversation| { conversation.conversation_id == "runtime-only-key" })
        );
        assert_eq!(
            state
                .response
                .current
                .points
                .iter()
                .map(|point| point.parallel_count)
                .sum::<i64>(),
            0
        );
    }

    #[test]
    fn parallel_work_projection_replays_runtime_entries_after_rebase() {
        let occurred_at = Utc::now();
        let mut old = parallel_work_materializer_state(
            "1d",
            occurred_at - ChronoDuration::minutes(2),
            0,
            None,
        );
        let mutation = RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "rebase-race-invoke",
                format_utc_iso(occurred_at),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("rebase-race-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        assert!(old.apply_runtime_mutation(&mutation).changed);

        let mut rebased = parallel_work_materializer_state(
            "1d",
            occurred_at - ChronoDuration::minutes(1),
            0,
            None,
        );
        assert!(rebased.replay_runtime_mutations(&old.runtime_mutations));
        assert!(
            rebased
                .response
                .current
                .conversations
                .iter()
                .any(|conversation| { conversation.conversation_id == "rebase-race-key" })
        );
    }

    #[test]
    fn parallel_work_projection_skips_runtime_entries_already_in_rebased_baseline() {
        let occurred_at = Utc::now();
        let mut old = parallel_work_materializer_state(
            "1d",
            occurred_at - ChronoDuration::minutes(2),
            0,
            None,
        );
        let mutation = RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "persisted-rebase-invoke",
                format_utc_iso(occurred_at),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("persisted-rebase-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        assert!(old.apply_runtime_mutation(&mutation).changed);

        let mut rebased = parallel_work_materializer_state(
            "1d",
            occurred_at - ChronoDuration::minutes(1),
            0,
            None,
        );
        for window in [
            &mut rebased.response.current,
            &mut rebased.response.minute7d,
            &mut rebased.response.hour30d,
            &mut rebased.response.day_all,
        ] {
            window.conversations = vec![ParallelWorkConversation {
                conversation_id: "persisted-rebase-key".to_string(),
                start: format_utc_iso(occurred_at),
                end: format_utc_iso(occurred_at + ChronoDuration::minutes(1)),
                request_count: 1,
            }];
        }
        rebased.baseline_response = rebased.response.clone();
        rebased.baseline_identities.insert(format!(
            "{}\0{}",
            mutation.identity.invoke_id, mutation.identity.occurred_at
        ));

        assert!(!rebased.replay_runtime_mutations(&old.runtime_mutations));
        assert_eq!(rebased.response.current.conversations[0].request_count, 1);
    }

    #[tokio::test]
    async fn parallel_work_reconcile_failure_schedules_runtime_recovery_retry() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: Some(77),
        };
        let topic_key = topic.cache_key().expect("parallel-work topic key");
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register parallel-work owner");
        hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("build initial parallel-work baseline");
        {
            let mut guard = hub.state.lock().await;
            let cached = guard.topics.get_mut(&topic_key).expect("cached topic");
            cached.dirty = true;
            cached.parallel_work_refresh_scheduled = true;
        }

        hub.schedule_parallel_work_reconcile_retry(state.clone(), &topic)
            .await;

        let guard = hub.state.lock().await;
        let cached = guard.topics.get(&topic_key).expect("cached topic");
        assert!(cached.dirty);
        assert!(!cached.parallel_work_refresh_scheduled);
        assert!(cached.runtime_topic_recovery_retry_at.is_some());
        assert!(guard.runtime_topic_recovery_running);
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn parallel_work_projection_replays_mutations_buffered_before_cold_baseline() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        };
        let topic_key = topic.cache_key().expect("parallel-work topic key");
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register parallel-work owner");
        let occurred_at = format_utc_iso(Utc::now());
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new("prebaseline-invoke", occurred_at),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("prebaseline-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }];

        hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
            .await
            .expect("buffer mutation before initial baseline");
        assert!(
            hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .contains_key(&topic_key)
        );

        hub.prepare_connection(state, vec![topic.descriptor()], Vec::new())
            .await
            .expect("build and replay parallel-work baseline");
        let guard = hub.state.lock().await;
        let Some(DashboardTopicMaterializer::ParallelWork { base }) =
            guard.topics[&topic_key].dashboard_materializer.as_ref()
        else {
            panic!("parallel-work topic must use a typed materializer");
        };
        assert!(
            base.lock()
                .expect("parallel-work materializer state lock")
                .response
                .current
                .conversations
                .iter()
                .any(|conversation| conversation.conversation_id == "prebaseline-key")
        );
        drop(guard);
        drop(lease);
    }

    #[tokio::test]
    async fn parallel_work_projection_discards_prebaseline_mutations_after_last_owner_leaves() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        };
        let topic_key = topic.cache_key().expect("parallel-work topic key");
        let mut lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register parallel-work owner");
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new(
                    "released-prebaseline-invoke",
                    format_utc_iso(Utc::now()),
                ),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("released-prebaseline-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }];

        hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
            .await
            .expect("buffer mutation before initial baseline");
        assert!(
            hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .contains_key(&topic_key)
        );

        let topic_keys = std::mem::take(&mut lease.topic_keys);
        let topic_names = std::mem::take(&mut lease.topic_names);
        hub.release_topic_subscribers(topic_keys, topic_names, lease.owns_dashboard_live)
            .await;
        drop(lease);

        assert!(
            !hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .contains_key(&topic_key)
        );

        hub.schedule_parallel_work_topic_projection(state, topic, &mutations)
            .await
            .expect("ignore mutation after final owner release");
        assert!(
            !hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .contains_key(&topic_key)
        );
    }

    #[tokio::test]
    async fn parallel_work_projection_retains_unknown_account_mutation_through_reconcile() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: Some(77),
        };
        let topic_key = topic.cache_key().expect("parallel-work topic key");
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register parallel-work owner");
        hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("build initial parallel-work baseline");
        let mutation = RuntimeInvocationMutation {
            identity: RuntimeInvocationIdentity::new(
                "unknown-account-invoke",
                format_utc_iso(Utc::now()),
            ),
            kind: RuntimeMutationKind::RuntimeUpsert,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: Some("unknown-account-key".to_string()),
            sticky_key: None,
            upstream_account_id: None,
        };
        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(mutation.clone()),
        }];

        hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
            .await
            .expect("schedule unknown-account reconcile");
        hub.refresh_topic_if_active(state.clone(), topic.clone(), true)
            .await
            .expect("reconcile unknown-account baseline");

        assert!(
            hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .get(&topic_key)
                .is_some_and(|pending| pending.contains_key(&format!(
                    "{}\0{}",
                    mutation.identity.invoke_id, mutation.identity.occurred_at
                )))
        );

        let removed = RuntimeInvocationMutation {
            identity: mutation.identity.clone(),
            kind: RuntimeMutationKind::RuntimeRemoved,
            row_id: None,
            is_terminal: false,
            prompt_cache_key: None,
            sticky_key: None,
            upstream_account_id: None,
        };
        let removals = [SequencedRuntimeMutation {
            sequence: 2,
            mutation: RuntimeMutation::Invocation(removed),
        }];
        hub.schedule_parallel_work_topic_projection(state.clone(), topic, &removals)
            .await
            .expect("discard removed unknown-account mutation");

        assert!(
            !hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .contains_key(&topic_key)
        );
        drop(lease);
    }

    #[tokio::test]
    async fn parallel_work_projection_replays_mutations_buffered_while_rebasing() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        };
        let topic_key = topic.cache_key().expect("parallel-work topic key");
        let lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register parallel-work owner");
        hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("build initial parallel-work baseline");
        {
            let mut guard = hub.state.lock().await;
            guard
                .topics
                .get_mut(&topic_key)
                .expect("cached topic")
                .dirty = true;
        }

        let mutations = [SequencedRuntimeMutation {
            sequence: 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new(
                    "rebase-buffered-invoke",
                    format_utc_iso(Utc::now()),
                ),
                kind: RuntimeMutationKind::RuntimeUpsert,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: Some("rebase-buffered-key".to_string()),
                sticky_key: None,
                upstream_account_id: None,
            }),
        }];
        hub.schedule_parallel_work_topic_projection(state.clone(), topic.clone(), &mutations)
            .await
            .expect("buffer mutation while rebase is in flight");
        assert!(
            hub.state
                .lock()
                .await
                .parallel_work_prebaseline_mutations
                .contains_key(&topic_key)
        );

        hub.refresh_topic_if_active(state, topic, true)
            .await
            .expect("rebuild parallel-work baseline")
            .expect("active owner receives rebuilt topic");
        let guard = hub.state.lock().await;
        let Some(DashboardTopicMaterializer::ParallelWork { base }) =
            guard.topics[&topic_key].dashboard_materializer.as_ref()
        else {
            panic!("parallel-work topic must retain a typed materializer");
        };
        assert!(
            base.lock()
                .expect("parallel-work materializer state lock")
                .response
                .current
                .conversations
                .iter()
                .any(|conversation| conversation.conversation_id == "rebase-buffered-key")
        );
        drop(guard);
        drop(lease);
    }

    #[test]
    fn dashboard_delivery_reconnect_churn_is_degraded() {
        let counters = DashboardDeliveryTopologyCounters::default();
        counters.record_reconnect_churn("dashboard.working-conversations.current");

        assert!(counters.has_degraded_signal());
    }

    #[test]
    fn dashboard_topic_classification_is_exhaustive_for_live_closed_and_cold_topics() {
        let hot = [
            SubscriptionTopic::DashboardActivityCurrent {
                range: "today".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                recent_limit: 16,
                include_accounts: true,
                include_recent: true,
            },
            SubscriptionTopic::DashboardWorkingConversationsCurrent {
                page_size: 16,
                recent_invocation_limit: 16,
                blocked_binding_upstream_account_id: None,
                blocked_binding_constraint_source: None,
            },
            SubscriptionTopic::ParallelWorkCurrent {
                range: "1d".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                bucket: Some("1m".to_string()),
                upstream_account_id: None,
            },
            SubscriptionTopic::TimeseriesOpenWindow {
                range: "1d".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                bucket: Some("1m".to_string()),
                settlement_hour: None,
                upstream_account_id: None,
            },
        ];
        for topic in hot {
            assert_eq!(topic.class(), SubscriptionTopicClass::HotProjection);
        }

        for topic in [
            SubscriptionTopic::DashboardActivityCurrent {
                range: "yesterday".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                recent_limit: 16,
                include_accounts: true,
                include_recent: true,
            },
            SubscriptionTopic::ParallelWorkCurrent {
                range: "yesterday".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                bucket: Some("1m".to_string()),
                upstream_account_id: None,
            },
        ] {
            assert_eq!(topic.class(), SubscriptionTopicClass::ClosedSnapshot);
        }

        assert_eq!(
            SubscriptionTopic::InvocationWindow {
                limit: 50,
                model: None,
                status: None,
            }
            .class(),
            SubscriptionTopicClass::BoundedColdHydrate,
        );
    }

    #[test]
    fn dashboard_hot_topic_health_reports_fallback_db_cadence_and_churn() {
        let counters = DashboardDeliveryTopologyCounters::default();
        counters.record_materialization("stats.parallel-work.current", true);
        counters.record_reconnect_churn("dashboard.working-conversations.current");
        let projection = DashboardRuntimeTopologyCounterSnapshot {
            current: DashboardProjectionSliceCounterSnapshot {
                cadence_miss_count: 2,
                ..Default::default()
            },
            ..Default::default()
        };

        let health =
            counters.hot_topic_health(projection, DashboardHotTopicRecoveryHealth::default());

        assert_eq!(health.state, "degraded");
        assert_eq!(health.activity.cadence_miss_count, 2);
        assert_eq!(health.parallel_work.generic_fallback_build_count, 1);
        assert_eq!(health.parallel_work.live_path_db_read_count, 1);
        assert_eq!(health.parallel_work.state, "degraded");
        assert_eq!(health.working_conversations.reconnect_churn_count, 1);
        assert_eq!(health.working_conversations.state, "degraded");
        assert_eq!(health.timeseries.topic_class, "hot_projection");
    }

    #[test]
    fn dashboard_hot_topic_health_attributes_cadence_misses_to_each_materializer() {
        let counters = DashboardDeliveryTopologyCounters::default();
        counters.record_cadence_miss("dashboard.working-conversations.current");
        counters.record_cadence_miss("stats.parallel-work.current");
        let projection = DashboardRuntimeTopologyCounterSnapshot {
            current: DashboardProjectionSliceCounterSnapshot {
                cadence_miss_count: 2,
                ..Default::default()
            },
            terminal: DashboardProjectionSliceCounterSnapshot {
                cadence_miss_count: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let health =
            counters.hot_topic_health(projection, DashboardHotTopicRecoveryHealth::default());

        assert_eq!(health.working_conversations.cadence_miss_count, 1);
        assert_eq!(health.working_conversations.state, "degraded");
        assert_eq!(health.parallel_work.cadence_miss_count, 1);
        assert_eq!(health.parallel_work.state, "degraded");
        assert_eq!(health.timeseries.cadence_miss_count, 5);
        assert_eq!(health.timeseries.state, "degraded");
        assert_eq!(health.state, "degraded");
    }

    #[test]
    fn dashboard_hot_topic_health_attributes_activity_and_summary_slice_dependencies() {
        let counters = DashboardDeliveryTopologyCounters::default();
        let projection = DashboardRuntimeTopologyCounterSnapshot {
            network: DashboardProjectionSliceCounterSnapshot {
                cadence_miss_count: 5,
                ..Default::default()
            },
            terminal: DashboardProjectionSliceCounterSnapshot {
                cadence_miss_count: 7,
                ..Default::default()
            },
            ..Default::default()
        };

        let health =
            counters.hot_topic_health(projection, DashboardHotTopicRecoveryHealth::default());

        assert_eq!(health.activity.cadence_miss_count, 12);
        assert_eq!(health.activity.state, "degraded");
        assert_eq!(health.summary.cadence_miss_count, 7);
        assert_eq!(health.summary.state, "degraded");
    }

    #[tokio::test]
    async fn dashboard_hot_topic_health_marks_cursor_gap_last_good_as_degraded() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = state.subscription_hub.clone();
        let topics = vec![
            SubscriptionTopic::DashboardWorkingConversationsCurrent {
                page_size: 20,
                recent_invocation_limit: 16,
                blocked_binding_upstream_account_id: None,
                blocked_binding_constraint_source: None,
            },
            SubscriptionTopic::ParallelWorkCurrent {
                range: "1d".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                bucket: Some("1m".to_string()),
                upstream_account_id: None,
            },
            SubscriptionTopic::TimeseriesOpenWindow {
                range: "1d".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                bucket: Some("1m".to_string()),
                settlement_hour: None,
                upstream_account_id: None,
            },
        ];
        {
            let mut guard = hub.state.lock().await;
            for topic in &topics {
                guard.topics.insert(
                    topic.cache_key().expect("hot topic key"),
                    seeded_cached_topic(topic.clone(), &[7], Utc::now()),
                );
            }
            // Keep the recovery worker parked so the health snapshot observes dirty last-good.
            guard.runtime_topic_recovery_running = true;
        }
        let lease = hub
            .register_topic_subscribers(&topics)
            .await
            .expect("register hot topic owners");

        hub.mark_runtime_mutation_gap_and_recover(state, 4, "cursor_gap")
            .await;
        let health = hub
            .dashboard_hot_topic_health(DashboardRuntimeTopologyCounterSnapshot::default())
            .await;

        assert_eq!(health.working_conversations.state, "degraded");
        assert_eq!(health.parallel_work.state, "degraded");
        assert_eq!(health.timeseries.state, "degraded");
        assert_eq!(health.state, "degraded");
        drop(lease);
    }

    #[test]
    fn hot_fallback_metrics_exclude_closed_dashboard_snapshots() {
        let closed_activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "yesterday".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let closed_summary = SubscriptionTopic::SummaryCurrent {
            window: "yesterday".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let closed_parallel = SubscriptionTopic::ParallelWorkCurrent {
            range: "yesterday".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        };
        let closed_timeseries = SubscriptionTopic::TimeseriesOpenWindow {
            range: "yesterday".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            upstream_account_id: None,
        };
        let open_parallel = SubscriptionTopic::ParallelWorkCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            upstream_account_id: None,
        };
        let open_timeseries = SubscriptionTopic::TimeseriesOpenWindow {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            upstream_account_id: None,
        };
        let mutation = RuntimeMutation::invocation(
            &dashboard_runtime_topology_live_record("2026-08-11 12:00:00"),
            RuntimeMutationKind::RuntimeUpsert,
        );

        for topic in [
            &closed_activity,
            &closed_summary,
            &closed_parallel,
            &closed_timeseries,
        ] {
            assert!(!topic.is_unmigrated_dashboard_hot_projection());
        }
        assert!(!open_parallel.is_unmigrated_dashboard_hot_projection());
        assert!(!open_timeseries.is_unmigrated_dashboard_hot_projection());
        assert!(!closed_parallel.is_affected_by_runtime_mutation(&mutation));
        assert!(!closed_timeseries.is_affected_by_runtime_mutation(&mutation));
        assert!(open_parallel.is_affected_by_runtime_mutation(&mutation));
        assert!(open_timeseries.is_affected_by_runtime_mutation(&mutation));
    }

    #[tokio::test]
    async fn inactive_topics_clear_dirty_when_rebuilt_payload_is_unchanged() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = SubscriptionHub::new();
        let topic = SubscriptionTopic::AppVersion;
        let descriptor = topic.descriptor();
        let topic_key = topic.cache_key().expect("topic key");

        let initial = hub
            .prepare_connection(state.clone(), vec![descriptor.clone()], Vec::new())
            .await
            .expect("prepare initial app version topic");
        let initial_cursor = initial.outcomes[0].cursor;
        hub.handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::Version {
                version: "next".to_string(),
            },
        )
        .await;
        {
            let guard = hub.state.lock().await;
            assert!(
                guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|cached| cached.dirty)
            );
        }

        let prepared = hub
            .prepare_connection(
                state,
                vec![descriptor],
                vec![SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: initial_cursor,
                    schema_epoch: topic.schema_epoch(),
                }],
            )
            .await
            .expect("reconnect should rebuild dirty topic");
        assert!(prepared.initial.is_empty());
        assert_eq!(
            prepared.outcomes[0].disposition,
            TopicInitDisposition::ResumeCaughtUp
        );
        assert_eq!(prepared.outcomes[0].cursor, initial_cursor);
        assert_eq!(prepared.outcomes[0].miss_reason, None);
        let guard = hub.state.lock().await;
        assert!(!guard.topics.get(&topic_key).expect("cached topic").dirty);
        assert!(
            guard
                .topics
                .get(&topic_key)
                .expect("cached topic")
                .replay_events
                .is_empty()
        );
    }

    #[tokio::test]
    async fn releasing_the_last_subscriber_marks_cached_topic_dirty() {
        let hub = SubscriptionHub::new();
        let topic = SubscriptionTopic::InvocationWindow {
            limit: 20,
            model: None,
            status: None,
        };
        let topic_key = topic.cache_key().expect("invocation topic key");
        hub.state.lock().await.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic, &[], Utc::now()),
        );
        {
            let mut guard = hub.state.lock().await;
            guard.active_topics.insert(
                topic_key.clone(),
                SubscriptionTopic::InvocationWindow {
                    limit: 20,
                    model: None,
                    status: None,
                },
            );
            guard.active_subscribers.insert(topic_key.clone(), 1);
        }

        hub.release_topic_subscribers(
            vec![topic_key.clone()],
            vec!["invocations.window".to_string()],
            false,
        )
        .await;

        assert!(
            hub.state
                .lock()
                .await
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.dirty),
            "a reconnect must rebuild cached data changed while no owner was subscribed"
        );
    }

    #[tokio::test]
    async fn topic_name_dirty_marker_invalidates_cached_quota_snapshot() {
        let hub = SubscriptionHub::new();
        let topic = SubscriptionTopic::QuotaCurrent;
        let topic_key = topic.cache_key().expect("quota topic key");
        hub.state.lock().await.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic, &[], Utc::now()),
        );

        hub.mark_topic_name_dirty("quota.current").await;

        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.dirty)
        );
    }

    #[tokio::test]
    async fn summary_live_overlay_updates_only_changed_fields() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = summary_topic();
        let topic_key = topic.cache_key().expect("topic key");
        let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
        cached.snapshot_payload = json!({
            "inProgressConversationCount": 0,
            "inProgressRetryConversationCount": 0,
            "inProgressAvgWaitMs": null,
            "inProgressPhaseCounts": {"queued": 0, "requesting": 0, "responding": 0}
        });
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);
        let mut receiver = hub.subscribe();
        let live = DashboardActivityLiveSnapshot {
            revision: 1,
            generated_at: "2026-07-24T00:00:00.000Z".to_string(),
            in_progress_invocation_count: 2,
            in_progress_phase_counts: InvocationPhaseCountsResponse {
                queued: 1,
                requesting: 1,
                responding: 0,
            },
            retry_invocation_count: 1,
            in_progress_wait_sum_ms: 80.0,
            in_progress_wait_sample_count: 2,
            network_live_bucket: None,
            network_realtime_rate: None,
            accounts: Vec::new(),
        };

        hub.apply_summary_live_overlay(&topic, live.clone())
            .await
            .expect("apply summary live overlay");
        let dispatch = receiver
            .recv()
            .await
            .expect("summary overlay should dispatch a changed payload");
        assert_eq!(hub.serialization_count(), 1);
        {
            let guard = hub.state.lock().await;
            let cached = guard.topics.get(&topic_key).expect("cached summary topic");
            let replay = cached.replay_events.back().expect("summary replay frame");
            assert!(Arc::ptr_eq(&cached.snapshot_frame, &replay.frame));
            assert!(Arc::ptr_eq(&cached.snapshot_frame, &dispatch.frame));
            assert_eq!(
                cached.snapshot_frame.fingerprint,
                dispatch.frame.fingerprint
            );
        }
        let dispatch_payload = dispatch.frame.payload_value();
        assert_eq!(dispatch_payload["inProgressConversationCount"], json!(2));
        assert_eq!(
            dispatch_payload["inProgressRetryConversationCount"],
            json!(1)
        );
        assert_eq!(dispatch_payload["inProgressAvgWaitMs"], json!(40.0));

        hub.apply_summary_live_overlay(&topic, live)
            .await
            .expect("reapply summary live overlay");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), receiver.recv())
                .await
                .is_err()
        );
        let guard = hub.state.lock().await;
        assert_eq!(
            guard.topics.get(&topic_key).expect("cached topic").cursor,
            1
        );
        assert_eq!(hub.serialization_count(), 1);
    }

    #[tokio::test]
    async fn additional_topic_owners_reuse_the_committed_serialized_frame() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = summary_topic();
        let topic_key = topic.cache_key().expect("topic key");
        hub.state.lock().await.topics.insert(
            topic_key.clone(),
            seeded_cached_topic(topic.clone(), &[], Utc::now()),
        );

        let first = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("first owner");
        let second = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("second owner");

        let guard = hub.state.lock().await;
        assert_eq!(guard.active_subscribers.get(&topic_key), Some(&2));
        assert_eq!(hub.serialization_count(), 0);
        drop(guard);
        drop(first);
        drop(second);
    }

    #[tokio::test]
    async fn closed_summary_topics_do_not_keep_live_snapshot_worker_active() {
        let hub = Arc::new(SubscriptionHub::new());
        let closed_topic = SubscriptionTopic::SummaryCurrent {
            window: "previous7d".to_string(),
            time_zone: "Asia/Shanghai".to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let closed_key = closed_topic.cache_key().expect("closed topic key");
        hub.state.lock().await.topics.insert(
            closed_key,
            seeded_cached_topic(closed_topic, &[], Utc::now()),
        );
        let _closed_lease = hub.register_test_topic_name("stats.summary.current").await;

        assert!(!hub.has_active_dashboard_activity_live_topic().await);
        assert!(!hub.has_active_dashboard_activity_live_topic_sync());

        let open_topic = summary_topic();
        let open_key = open_topic.cache_key().expect("open topic key");
        hub.state
            .lock()
            .await
            .topics
            .insert(open_key, seeded_cached_topic(open_topic, &[], Utc::now()));
        let _open_lease = hub.register_test_topic_name("stats.summary.current").await;

        assert!(hub.has_active_dashboard_activity_live_topic().await);
        assert!(hub.has_active_dashboard_activity_live_topic_sync());
    }

    #[test]
    fn closed_summary_topics_use_calendar_rollover_push_cadence() {
        for window in ["yesterday", "previous7d"] {
            let topic = SubscriptionTopic::SummaryCurrent {
                window: window.to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                limit: None,
                upstream_account_id: None,
            };

            assert!(topic.uses_server_push_cadence(RuntimeProjectionMode::Auto));
            assert!(
                subscription_calendar_rollover_delay(&topic) <= Duration::from_secs(24 * 60 * 60)
            );
        }
    }

    #[test]
    fn closed_summary_rollover_uses_next_local_midnight_across_dst() {
        let topic = SubscriptionTopic::SummaryCurrent {
            window: "previous7d".to_string(),
            time_zone: "America/New_York".to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let reporting_tz = parse_reporting_tz(Some("America/New_York")).expect("valid timezone");
        let before_fall_back = Utc
            .with_ymd_and_hms(2026, 11, 1, 5, 30, 0)
            .single()
            .expect("valid UTC instant");

        assert_eq!(
            subscription_calendar_rollover_delay_at(&topic, before_fall_back, reporting_tz),
            Duration::from_secs(23 * 60 * 60 + 30 * 60)
        );
    }

    #[tokio::test]
    async fn network_timeseries_topics_keep_live_snapshot_worker_active() {
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            upstream_account_id: None,
        };
        let topic_key = topic.cache_key().expect("network topic key");
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key, seeded_cached_topic(topic, &[], Utc::now()));
        let _lease = hub
            .register_test_topic_name("dashboard.network-timeseries.window")
            .await;

        assert!(hub.has_active_dashboard_activity_live_topic().await);
        assert!(hub.has_active_dashboard_activity_live_topic_sync());
    }

    #[tokio::test]
    async fn cold_network_only_slice_materializes_without_sqlite() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        state
            .proxy_runtime_invocations
            .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
            .expect("bind network cache");
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            upstream_account_id: None,
        };
        let topic_key = topic.cache_key().expect("network topic key");
        hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("prepare network topic");
        let _lease = hub
            .register_test_topic_name("dashboard.network-timeseries.window")
            .await;
        let cursor_before = hub
            .state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .expect("cached network topic")
            .cursor;
        state.dashboard_network_speed_cache.record_request_bytes(
            "cold-network-only",
            &crate::proxy::shanghai_now_string(),
            None,
            Some("api.openai.com"),
            512,
            Utc::now(),
        );
        state.dashboard_network_speed_cache.record_request_bytes(
            "cold-network-account",
            &crate::proxy::shanghai_now_string(),
            Some(42),
            Some("api.openai.com"),
            256,
            Utc::now(),
        );
        let slice = state
            .proxy_runtime_invocations
            .capture_network_slice()
            .expect("capture network slice")
            .slice;
        state.pool.close().await;

        hub.handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardNetworkSlice {
                slice: Box::new(slice.clone()),
            },
        )
        .await;

        let cursor_after = hub
            .state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .expect("cached network topic")
            .cursor;
        assert_eq!(cursor_after, cursor_before + 1);
        let guard = hub.state.lock().await;
        let payload = guard
            .topics
            .get(&topic_key)
            .map(|cached| cached.snapshot_frame.payload_value())
            .expect("cached network frame");
        let live_point = payload
            .get("points")
            .and_then(Value::as_array)
            .and_then(|points| {
                points.iter().find(|point| {
                    point
                        .get("isLiveBucket")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
            })
            .expect("global live point");
        assert_eq!(
            live_point.get("uploadBytes").and_then(Value::as_i64),
            Some(768)
        );
        drop(guard);

        let delivery_before = hub.dashboard_topology_counters();
        hub.handle_internal_broadcast(
            state,
            BroadcastPayload::DashboardNetworkSlice {
                slice: Box::new(slice),
            },
        )
        .await;
        let delivery_after = hub.dashboard_topology_counters();
        let cursor_after_unchanged = hub
            .state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .expect("cached network topic")
            .cursor;
        assert_eq!(cursor_after_unchanged, cursor_after);
        assert_eq!(
            delivery_after.network_timeseries.materialization_count,
            delivery_before.network_timeseries.materialization_count,
            "unchanged network revisions must not rematerialize a topic",
        );
        assert_eq!(
            delivery_after.network_timeseries.serialization_count,
            delivery_before.network_timeseries.serialization_count,
            "unchanged network revisions must not serialize a topic",
        );
        assert_eq!(delivery_after.network_timeseries.payload_clone_count, 0);
    }

    #[tokio::test]
    async fn terminal_slice_materializes_activity_and_summary_without_live_sqlite() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let topics = vec![activity.clone(), summary.clone()];
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(&topics)
            .await
            .expect("register active Dashboard topics");
        state
            .subscription_hub
            .prepare_connection(
                state.clone(),
                topics.iter().map(SubscriptionTopic::descriptor).collect(),
                Vec::new(),
            )
            .await
            .expect("prepare Dashboard topic bases");

        let activity_key = activity.cache_key().expect("activity topic key");
        let summary_key = summary.cache_key().expect("summary topic key");
        let (activity_cursor_before, summary_cursor_before) = {
            let guard = state.subscription_hub.state.lock().await;
            (
                guard.topics[&activity_key].cursor,
                guard.topics[&summary_key].cursor,
            )
        };
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.invoke_id = "dashboard-runtime-terminal-materialization".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        terminal.prompt_cache_key = Some("terminal-materialized-recent".to_string());
        terminal.request_model = Some("gpt-5".to_string());
        terminal.response_model = Some("gpt-5".to_string());
        terminal.t_total_ms = Some(3_450.0);
        terminal.t_req_read_ms = Some(10.0);
        terminal.t_req_parse_ms = Some(12.0);
        terminal.t_upstream_connect_ms = Some(18.0);
        terminal.t_upstream_ttfb_ms = Some(508.0);
        terminal.first_token_ms = Some(650.0);
        terminal.t_upstream_stream_ms = Some(2_800.0);
        let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal).await;
        let delta = outcome.terminal_delta.expect("accepted terminal delta");
        state.pool.close().await;

        state
            .subscription_hub
            .handle_internal_broadcast(
                state.clone(),
                BroadcastPayload::DashboardTerminalSlice {
                    slice: Box::new(DashboardTerminalProjectionSlice {
                        revision: 1,
                        deltas: vec![delta],
                    }),
                },
            )
            .await;

        let guard = state.subscription_hub.state.lock().await;
        let activity_cached = &guard.topics[&activity_key];
        let summary_cached = &guard.topics[&summary_key];
        assert_eq!(activity_cached.cursor, activity_cursor_before + 1);
        assert_eq!(summary_cached.cursor, summary_cursor_before + 1);
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["summary"]["stats"]["totalCount"],
            json!(1)
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["accounts"][0]["upstreamAccountId"],
            json!(42)
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["accounts"][0]["totalTokens"],
            json!(42)
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["summary"]["modelPerformance"]["total"]
                ["cumulativeUsageDurationMs"],
            json!(3_450.0)
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["accounts"][0]["firstTokenAvgMs"],
            json!(650.0)
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["accounts"][0]["avgTotalMs"],
            json!(3_450.0)
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["accounts"][0]["modelPerformance"]["models"]
                [0]["model"],
            json!("gpt-5")
        );
        assert_eq!(
            activity_cached.snapshot_frame.payload_value()["accounts"][0]["recentInvocations"][0]["invokeId"],
            json!("dashboard-runtime-terminal-materialization")
        );
        assert_eq!(
            summary_cached.snapshot_frame.payload_value()["totalCount"],
            json!(1)
        );
        drop(guard);

        state
            .subscription_hub
            .handle_internal_broadcast(
                state.clone(),
                BroadcastPayload::DashboardTerminalSlice {
                    slice: Box::new(DashboardTerminalProjectionSlice {
                        revision: 1,
                        deltas: Vec::new(),
                    }),
                },
            )
            .await;
        let guard = state.subscription_hub.state.lock().await;
        assert_eq!(
            guard.topics[&activity_key].cursor,
            activity_cursor_before + 1,
            "an unchanged terminal revision must not advance the activity cursor",
        );
        assert_eq!(
            guard.topics[&summary_key].cursor,
            summary_cursor_before + 1,
            "an unchanged terminal revision must not advance the summary cursor",
        );
    }

    #[tokio::test]
    async fn activity_materializer_skips_terminal_delta_already_folded_into_base() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.invoke_id = "dashboard-runtime-folded-terminal".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal).await;
        let delta = outcome.terminal_delta.expect("accepted terminal delta");

        let payload = activity
            .build_cached_payload(state.clone())
            .await
            .expect("build typed activity base")
            .serialize(
                None,
                None,
                Some(&DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![delta],
                }),
            )
            .expect("serialize folded activity base");
        let payload: Value = serde_json::from_slice(&payload).expect("activity payload JSON");
        assert_eq!(payload["summary"]["stats"]["totalCount"], json!(1));
        assert_eq!(payload["summary"]["stats"]["totalTokens"], json!(42));
    }

    #[tokio::test]
    async fn activity_materializer_skips_terminal_delta_already_persisted_in_base() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 0;
        terminal.invoke_id = "dashboard-runtime-persisted-activity-terminal".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
            .await
            .terminal_delta
            .expect("accepted pending terminal delta");

        terminal.id = 748_004;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, total_tokens, output_tokens, cost,
                status, payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', '{}')
            "#,
        )
        .bind(terminal.id)
        .bind(&terminal.invoke_id)
        .bind(&terminal.occurred_at)
        .bind(&terminal.source)
        .bind(terminal.model.as_deref())
        .bind(terminal.total_tokens)
        .bind(terminal.output_tokens)
        .bind(terminal.cost)
        .bind(terminal.status.as_deref())
        .execute(&state.pool)
        .await
        .expect("persist terminal baseline row");

        let base = build_dashboard_activity_topic_materialized_base(
            state.as_ref(),
            &DashboardActivityQuery {
                range: "today".to_string(),
                recent_limit: Some(16),
                time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
                include_accounts: true,
                include_recent: Some(true),
            },
        )
        .await
        .expect("build typed activity base");
        assert_eq!(base.response().terminal_sequence, delta.terminal_sequence);
        let payload = DashboardTopicMaterializer::Activity {
            base: Arc::new(StdMutex::new(DashboardActivityMaterializerState::new(base))),
            reporting_tz: Shanghai,
            source_scope: InvocationSourceScope::ProxyOnly,
        }
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: vec![delta],
            }),
        )
        .expect("serialize persisted activity base");
        let payload: Value = serde_json::from_slice(&payload).expect("activity payload JSON");
        assert_eq!(payload["summary"]["stats"]["totalCount"], json!(1));
        assert_eq!(payload["summary"]["stats"]["totalTokens"], json!(42));
    }

    #[tokio::test]
    async fn activity_materializer_handles_maximum_distinct_account_terminal_slice() {
        let delta_state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let base_state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let mut base = build_dashboard_activity_topic_materialized_base(
            base_state.as_ref(),
            &DashboardActivityQuery {
                range: "today".to_string(),
                recent_limit: Some(16),
                time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
                include_accounts: true,
                include_recent: Some(false),
            },
        )
        .await
        .expect("build typed activity base");
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut deltas = Vec::with_capacity(10_000);
        for account_id in 1..=10_000_i64 {
            let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
            terminal.id = 0;
            terminal.invoke_id = format!("dashboard-runtime-terminal-account-{account_id}");
            terminal.upstream_account_id = Some(account_id);
            terminal.upstream_account_name = Some(format!("Account {account_id}"));
            terminal.status = Some("success".to_string());
            terminal.live_phase = None;
            terminal.total_tokens = Some(1);
            terminal.output_tokens = Some(1);
            deltas.push(
                apply_dashboard_activity_terminal_record(delta_state.as_ref(), &terminal)
                    .await
                    .terminal_delta
                    .expect("accepted distinct terminal delta"),
            );
        }

        base.apply_terminal_slice(
            Shanghai,
            InvocationSourceScope::ProxyOnly,
            &DashboardTerminalProjectionSlice {
                revision: 1,
                deltas,
            },
        );

        assert_eq!(base.response().summary.stats.total_count, 10_000);
        assert_eq!(
            base.response().accounts.as_ref().map(Vec::len),
            Some(10_000),
            "each distinct account should be materialized once",
        );
    }

    #[tokio::test]
    async fn terminal_materializers_detect_stale_moving_window_bases() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let timeseries = SubscriptionTopic::TimeseriesOpenWindow {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            upstream_account_id: None,
        };
        let activity_materializer = activity
            .build_cached_payload(state.clone())
            .await
            .expect("build activity base")
            .dashboard_materializer()
            .expect("activity typed materializer");
        let summary_materializer = summary
            .build_cached_payload(state.clone())
            .await
            .expect("build summary base")
            .dashboard_materializer()
            .expect("summary typed materializer");
        let timeseries_materializer = timeseries
            .build_cached_payload(state)
            .await
            .expect("build timeseries base")
            .dashboard_materializer()
            .expect("timeseries typed materializer");

        assert!(
            !activity_materializer.requires_terminal_window_rebase(),
            "a freshly built activity base must remain on the terminal delivery path",
        );
        assert!(
            !summary_materializer.requires_terminal_window_rebase(),
            "a freshly built duration base must wait for the reconcile boundary",
        );
        assert!(
            !timeseries_materializer.requires_terminal_window_rebase(),
            "a freshly built timeseries base must wait for the reconcile boundary",
        );

        let DashboardTopicMaterializer::Activity { base, .. } = &activity_materializer else {
            panic!("expected activity materializer");
        };
        base.lock()
            .expect("activity materializer state lock")
            .rebase_range_start = Some(Utc::now() - ChronoDuration::days(1));
        let DashboardTopicMaterializer::Summary { base, .. } = &summary_materializer else {
            panic!("expected summary materializer");
        };
        base.lock()
            .expect("summary materializer state lock")
            .range_start = Some(Utc::now() - ChronoDuration::days(2));
        let DashboardTopicMaterializer::Timeseries { base, .. } = &timeseries_materializer else {
            panic!("expected timeseries materializer");
        };
        base.lock()
            .expect("timeseries materializer state lock")
            .set_range_start_for_test(
                Utc::now()
                    - ChronoDuration::days(1)
                    - ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64),
            );

        assert!(activity_materializer.requires_terminal_window_rebase());
        assert!(summary_materializer.requires_terminal_window_rebase());
        assert!(timeseries_materializer.requires_terminal_window_rebase());
    }

    #[tokio::test]
    async fn terminal_slice_keeps_fresh_moving_summary_base_off_sqlite() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&summary))
            .await
            .expect("register active summary topic");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
            .await
            .expect("prepare moving summary base");
        let summary_key = summary.cache_key().expect("summary topic key");
        state.pool.close().await;

        state
            .subscription_hub
            .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: Vec::new(),
            })
            .await;

        let guard = state.subscription_hub.state.lock().await;
        let cached = &guard.topics[&summary_key];
        assert!(
            !cached.dirty,
            "a fresh duration base must not schedule a SQLite rebase from terminal delivery",
        );
        assert!(
            !cached
                .dashboard_materializer
                .as_ref()
                .expect("typed summary materializer")
                .requires_terminal_window_rebase(),
        );
    }

    #[tokio::test]
    async fn terminal_slice_keeps_fresh_rolling_activity_base_off_sqlite() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&activity))
            .await
            .expect("register active rolling activity topic");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![activity.descriptor()], Vec::new())
            .await
            .expect("prepare rolling activity base");
        let activity_key = activity.cache_key().expect("activity topic key");
        {
            let guard = state.subscription_hub.state.lock().await;
            let DashboardTopicMaterializer::Activity { base, .. } = guard.topics[&activity_key]
                .dashboard_materializer
                .as_ref()
                .expect("typed activity materializer")
            else {
                panic!("expected activity materializer");
            };
            let mut base = base.lock().expect("activity materializer state lock");
            let current_start = base.rebase_range_start.expect("activity base range start");
            base.rebase_range_start = Some(current_start - ChronoDuration::seconds(1));
        }
        state.pool.close().await;

        state
            .subscription_hub
            .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: Vec::new(),
            })
            .await;

        let guard = state.subscription_hub.state.lock().await;
        let cached = &guard.topics[&activity_key];
        assert!(
            !cached.dirty,
            "a fresh rolling activity base must not schedule a SQLite rebase from terminal delivery",
        );
        assert!(
            !cached
                .dashboard_materializer
                .as_ref()
                .expect("typed activity materializer")
                .requires_terminal_window_rebase(),
        );
    }

    #[tokio::test]
    async fn rolling_activity_terminal_slice_preserves_rebase_anchor() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let materializer = activity
            .build_cached_payload(state.clone())
            .await
            .expect("build rolling activity base")
            .dashboard_materializer()
            .expect("typed activity materializer");
        let DashboardTopicMaterializer::Activity { base, .. } = &materializer else {
            panic!("expected activity materializer");
        };
        let stale_anchor = resolve_dashboard_activity_cached_range("1d", Shanghai)
            .expect("rolling activity range")
            .start
            - ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64);
        base.lock()
            .expect("activity materializer state lock")
            .rebase_range_start = Some(stale_anchor);

        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 0;
        terminal.invoke_id = "dashboard-runtime-rolling-activity-anchor".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
            .await
            .terminal_delta
            .expect("accepted terminal delta");

        materializer
            .serialize(
                None,
                None,
                Some(&DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![delta.clone()],
                }),
            )
            .expect("apply nonempty terminal slice");

        let base = base.lock().expect("activity materializer state lock");
        assert_eq!(base.rebase_range_start, Some(stale_anchor));
        assert_eq!(
            base.base.response().terminal_sequence,
            delta.terminal_sequence
        );
        drop(base);
        assert!(
            materializer.requires_terminal_window_rebase(),
            "terminal delivery must not reset a rolling activity rebase boundary",
        );
    }

    #[tokio::test]
    async fn failed_runtime_window_rebase_retains_an_isolated_last_good_frame() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&summary))
            .await
            .expect("register active summary topic");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
            .await
            .expect("prepare moving summary base");
        let summary_key = summary.cache_key().expect("summary topic key");
        {
            let guard = state.subscription_hub.state.lock().await;
            let DashboardTopicMaterializer::Summary { base, .. } = guard.topics[&summary_key]
                .dashboard_materializer
                .as_ref()
                .expect("typed summary materializer")
            else {
                panic!("expected summary materializer");
            };
            base.lock()
                .expect("summary materializer state lock")
                .range_start = Some(Utc::now() - ChronoDuration::days(2));
        }
        state.pool.close().await;

        state
            .subscription_hub
            .materialize_dashboard_terminal_slice(DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: Vec::new(),
            })
            .await;

        assert!(
            state.subscription_hub.state.lock().await.topics[&summary_key].dirty,
            "terminal delivery must isolate a stale base without starting a database rebase",
        );

        state
            .subscription_hub
            .reconcile_dashboard_terminal_window_bases(state.clone())
            .await;
        let guard = state.subscription_hub.state.lock().await;
        let cached = &guard.topics[&summary_key];
        assert!(
            cached.dirty && cached.refresh_scheduled,
            "a failed rebase must preserve its last-good frame and isolate every slice",
        );
        assert!(
            cached
                .dashboard_materializer
                .as_ref()
                .expect("typed summary materializer")
                .requires_terminal_window_rebase(),
            "the next runtime reconcile must retry the unresolved window rebase",
        );
    }

    #[tokio::test]
    async fn runtime_reconcile_rebases_stale_activity_and_summary_window_bases() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let activity = SubscriptionTopic::DashboardActivityCurrent {
            range: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            recent_limit: 16,
            include_accounts: true,
            include_recent: true,
        };
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let topics = vec![activity.clone(), summary.clone()];
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(&topics)
            .await
            .expect("register active Dashboard topics");
        state
            .subscription_hub
            .prepare_connection(
                state.clone(),
                topics.iter().map(SubscriptionTopic::descriptor).collect(),
                Vec::new(),
            )
            .await
            .expect("prepare Dashboard topic bases");
        let summary_key = summary.cache_key().expect("summary topic key");
        let summary_cursor_before = {
            let mut guard = state.subscription_hub.state.lock().await;
            let activity_cached = guard
                .topics
                .get_mut(&activity.cache_key().expect("activity topic key"))
                .expect("activity cache entry");
            let DashboardTopicMaterializer::Activity { base, .. } = activity_cached
                .dashboard_materializer
                .as_ref()
                .expect("activity materializer")
            else {
                panic!("expected activity materializer");
            };
            base.lock()
                .expect("activity materializer state lock")
                .rebase_range_start = Some(Utc::now() - ChronoDuration::days(1));
            let summary_cached = guard
                .topics
                .get_mut(&summary_key)
                .expect("summary cache entry");
            let DashboardTopicMaterializer::Summary { base, .. } = summary_cached
                .dashboard_materializer
                .as_ref()
                .expect("summary materializer")
            else {
                panic!("expected summary materializer");
            };
            base.lock()
                .expect("summary materializer state lock")
                .range_start = Some(Utc::now() - ChronoDuration::days(2));
            summary_cached.cursor
        };

        state
            .subscription_hub
            .reconcile_dashboard_terminal_window_bases(state.clone())
            .await;
        let guard = state.subscription_hub.state.lock().await;
        for topic_key in [
            activity.cache_key().expect("activity topic key"),
            summary_key.clone(),
        ] {
            let cached = &guard.topics[&topic_key];
            assert!(
                !cached.dirty
                    && !cached
                        .dashboard_materializer
                        .as_ref()
                        .expect("typed Dashboard materializer")
                        .requires_terminal_window_rebase(),
                "runtime reconciliation should replace stale terminal-window bases",
            );
        }
        assert_eq!(
            guard.topics[&summary_key].cursor, summary_cursor_before,
            "a byte-identical summary rebase should retain its shared frame cursor",
        );
    }

    #[tokio::test]
    async fn runtime_reconcile_marks_inactive_stale_window_bases_dirty_for_reconnect() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let initial_lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&summary))
            .await
            .expect("register initial summary owner");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
            .await
            .expect("prepare moving summary base");
        let summary_key = summary.cache_key().expect("summary topic key");
        {
            let guard = state.subscription_hub.state.lock().await;
            let DashboardTopicMaterializer::Summary { base, .. } = guard.topics[&summary_key]
                .dashboard_materializer
                .as_ref()
                .expect("typed summary materializer")
            else {
                panic!("expected summary materializer");
            };
            base.lock()
                .expect("summary materializer state lock")
                .range_start = Some(Utc::now() - ChronoDuration::days(2));
        }
        drop(initial_lease);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !state
                    .subscription_hub
                    .state
                    .lock()
                    .await
                    .active_subscribers
                    .contains_key(&summary_key)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("initial owner should release before the reconcile scan");

        state
            .subscription_hub
            .reconcile_dashboard_terminal_window_bases(state.clone())
            .await;
        {
            let guard = state.subscription_hub.state.lock().await;
            let cached = &guard.topics[&summary_key];
            assert!(
                cached.dirty && !cached.refresh_scheduled,
                "an inactive stale base must defer its authoritative rebuild to reconnect",
            );
        }

        let _returned_lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&summary))
            .await
            .expect("register returning summary owner");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![summary.descriptor()], Vec::new())
            .await
            .expect("rebuild stale summary base on reconnect");
        let guard = state.subscription_hub.state.lock().await;
        let cached = &guard.topics[&summary_key];
        assert!(
            !cached.dirty
                && !cached
                    .dashboard_materializer
                    .as_ref()
                    .expect("typed summary materializer")
                    .requires_terminal_window_rebase(),
            "reconnecting must receive an authoritative moving-window base",
        );
    }

    #[tokio::test]
    async fn summary_topic_base_preserves_pending_terminal_overlay() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 0;
        terminal.invoke_id = "dashboard-runtime-summary-pending-terminal".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
            .await
            .terminal_delta
            .expect("accepted pending terminal delta");

        let query = SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some(SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string()),
            upstream_account_id: None,
        };
        let summary_window = parse_summary_window(&query, state.config.list_limit_max as i64)
            .expect("summary window");
        let SummaryTopicTerminalConsistentBase {
            mut response,
            pending_terminal_deltas,
            terminal_sequence,
        } = build_summary_topic_terminal_consistent_base(state.as_ref(), &query)
            .await
            .expect("build terminal-consistent summary base");
        assert_eq!(terminal_sequence, delta.terminal_sequence);
        assert_eq!(pending_terminal_deltas.len(), 1);

        let initial_slice = DashboardTerminalProjectionSlice {
            revision: 0,
            deltas: pending_terminal_deltas,
        };
        let mut replayed_terminal_sequence = 0;
        apply_dashboard_terminal_slice_to_summary_response(
            &mut response,
            &mut replayed_terminal_sequence,
            &summary_window,
            Shanghai,
            InvocationSourceScope::ProxyOnly,
            None,
            &initial_slice,
        );
        let payload = DashboardTopicMaterializer::Summary {
            base: Arc::new(StdMutex::new(DashboardSummaryMaterializerState::new(
                response,
                terminal_sequence,
                summary_window_range(&summary_window, Shanghai, Utc::now())
                    .expect("summary range")
                    .map(|(start, _)| start),
            ))),
            window: summary_window,
            reporting_tz: Shanghai,
            source_scope: InvocationSourceScope::ProxyOnly,
            upstream_account_id: None,
        }
        .serialize(
            None,
            None,
            Some(&DashboardTerminalProjectionSlice {
                revision: 1,
                deltas: vec![delta],
            }),
        )
        .expect("serialize summary terminal base");
        let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
        assert_eq!(payload["totalCount"], json!(1));
        assert_eq!(payload["totalTokens"], json!(42));
    }

    #[tokio::test]
    async fn timeseries_topic_base_preserves_pending_terminal_overlay() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 0;
        terminal.invoke_id = "dashboard-runtime-timeseries-pending-terminal".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
            .await
            .terminal_delta
            .expect("accepted pending terminal delta");

        let timeseries = SubscriptionTopic::TimeseriesOpenWindow {
            range: "1d".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            upstream_account_id: None,
        };
        let materializer = timeseries
            .build_cached_payload(state)
            .await
            .expect("build terminal-consistent timeseries base")
            .dashboard_materializer()
            .expect("typed timeseries materializer");
        let initial_payload: Value = serde_json::from_slice(
            &materializer
                .serialize(None, None, None)
                .expect("serialize materialized timeseries base"),
        )
        .expect("timeseries payload JSON");

        assert_eq!(
            initial_payload["points"]
                .as_array()
                .expect("timeseries points")
                .iter()
                .map(|point| point["totalCount"].as_i64().unwrap_or_default())
                .sum::<i64>(),
            1,
            "the typed baseline must include the terminal delta before SQLite persistence",
        );

        let replayed_payload: Value = serde_json::from_slice(
            &materializer
                .serialize(
                    None,
                    None,
                    Some(&DashboardTerminalProjectionSlice {
                        revision: 1,
                        deltas: vec![delta],
                    }),
                )
                .expect("skip the terminal delta already folded into the base"),
        )
        .expect("timeseries replay payload JSON");
        assert_eq!(
            replayed_payload["points"]
                .as_array()
                .expect("timeseries points")
                .iter()
                .map(|point| point["totalCount"].as_i64().unwrap_or_default())
                .sum::<i64>(),
            1,
            "the sequence watermark must reject the pending terminal replay",
        );
    }

    #[tokio::test]
    async fn summary_materializer_skips_terminal_delta_already_folded_into_base() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let summary = SubscriptionTopic::SummaryCurrent {
            window: "today".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            limit: None,
            upstream_account_id: None,
        };
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut terminal = dashboard_runtime_topology_live_record(&occurred_at);
        terminal.id = 748_002;
        terminal.invoke_id = "dashboard-runtime-summary-folded-terminal".to_string();
        terminal.status = Some("success".to_string());
        terminal.live_phase = None;
        terminal.total_tokens = Some(42);
        terminal.output_tokens = Some(16);
        terminal.cost = Some(0.25);
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, model, total_tokens, output_tokens, cost,
                status, payload, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', '{}')
            "#,
        )
        .bind(terminal.id)
        .bind(&terminal.invoke_id)
        .bind(&terminal.occurred_at)
        .bind(&terminal.source)
        .bind(terminal.model.as_deref())
        .bind(terminal.total_tokens)
        .bind(terminal.output_tokens)
        .bind(terminal.cost)
        .bind(terminal.status.as_deref())
        .execute(&state.pool)
        .await
        .expect("persist terminal baseline row");
        let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal).await;
        let delta = outcome
            .terminal_delta
            .expect("accepted persisted terminal delta");

        let payload = summary
            .build_cached_payload(state.clone())
            .await
            .expect("build typed summary base")
            .serialize(
                None,
                None,
                Some(&DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![delta],
                }),
            )
            .expect("serialize folded summary base");
        let payload: Value = serde_json::from_slice(&payload).expect("summary payload JSON");
        assert_eq!(payload["totalCount"], json!(1));
        assert_eq!(payload["totalTokens"], json!(42));
    }

    #[tokio::test]
    async fn dashboard_materialization_rejects_out_of_order_network_revisions() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        state
            .proxy_runtime_invocations
            .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
            .expect("bind network cache");
        let hub = state.subscription_hub.clone();
        let topic = SubscriptionTopic::DashboardNetworkRecentCurrent;
        let topic_key = topic.cache_key().expect("network recent topic key");
        let _lease = hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register network recent topic");
        hub.prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("prepare network recent topic");
        let cursor_before = hub
            .state
            .lock()
            .await
            .topics
            .get(&topic_key)
            .expect("cached network recent topic")
            .cursor;

        let older = state
            .proxy_runtime_invocations
            .capture_network_slice()
            .expect("capture older network slice")
            .slice;
        let mut newer = older.clone();
        newer.revision = older.revision.saturating_add(1);
        newer.recent.range_end = "2026-08-06T00:00:01.000Z".to_string();

        let (stale_pending, stale_payload) = {
            let mut guard = hub.state.lock().await;
            guard.dashboard_network_slice = Some(Arc::new(older.clone()));
            let pending = collect_pending_dashboard_topic_materializations(&mut guard)
                .into_iter()
                .find(|pending| pending.topic_key == topic_key)
                .expect("pending stale network revision");
            let payload = pending
                .materializer
                .serialize(
                    guard.dashboard_current_slice.as_deref(),
                    guard.dashboard_network_slice.as_deref(),
                    guard.dashboard_terminal_slice.as_deref(),
                )
                .expect("serialize stale network revision");
            (pending, payload)
        };
        state.pool.close().await;

        hub.materialize_dashboard_network_slice(newer.clone()).await;
        hub.commit_dashboard_materialized_frame(stale_pending, stale_payload)
            .await
            .expect("stale materialization commit should be harmless");
        hub.materialize_dashboard_network_slice(older).await;

        let guard = hub.state.lock().await;
        assert_eq!(
            guard
                .dashboard_network_slice
                .as_ref()
                .map(|slice| slice.revision),
            Some(newer.revision),
            "a late slice must not regress the hub dependency revision",
        );
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("cached network recent topic");
        assert_eq!(
            cached.dashboard_materialized_revision,
            Some(DashboardTopicRevision {
                base_revision: cached.dashboard_base_revision,
                current_revision: None,
                network_revision: Some(newer.revision),
                terminal_revision: None,
            }),
            "a delayed frame must not replace the latest materialized revision",
        );
        assert_eq!(
            cached.cursor,
            cursor_before + 1,
            "only the newest revision should advance the SSE cursor",
        );
        assert_eq!(
            cached.snapshot_frame.payload_value()["rangeEnd"],
            json!(newer.recent.range_end),
        );
    }

    fn seeded_cached_topic(
        topic: SubscriptionTopic,
        cursors: &[u64],
        emitted_at: DateTime<Utc>,
    ) -> CachedSubscriptionTopic {
        let descriptor = topic.descriptor();
        let schema_epoch = topic.schema_epoch();
        let replay_events = cursors
            .iter()
            .map(|cursor| test_replay_event(&descriptor, &schema_epoch, *cursor, emitted_at, 32))
            .collect::<VecDeque<_>>();
        let replay_bytes = replay_events.iter().map(|event| event.bytes).sum::<usize>();
        let cursor = cursors.last().copied().unwrap_or(0);

        let snapshot_payload = json!({ "cursor": cursor });
        let snapshot_frame = Arc::new(
            serialize_topic_frame(
                descriptor.clone(),
                topic.cache_key().expect("seeded topic key"),
                schema_epoch.clone(),
                cursor,
                serde_json::to_vec(&snapshot_payload).expect("seeded payload"),
            )
            .expect("seeded frame"),
        );
        CachedSubscriptionTopic {
            topic,
            descriptor,
            schema_epoch,
            cursor,
            snapshot_built_at: Instant::now(),
            refresh_scheduled: false,
            conversation_overview_refresh_scheduled: false,
            conversation_overview_refresh_in_flight: false,
            conversation_overview_refresh_pending: false,
            dirty: false,
            runtime_topic_recovery_generation: 0,
            runtime_topic_recovery_retry_at: None,
            summary_refresh_scheduled: false,
            summary_refresh_in_flight: false,
            summary_pending_event_count: 0,
            summary_retry_backoff_ms: 0,
            parallel_work_refresh_scheduled: false,
            prompt_cache_refresh_scheduled: false,
            prompt_cache_reconcile_scheduled: false,
            prompt_cache_key_hydration_scheduled: false,
            prompt_cache_pending_records: BTreeMap::new(),
            prompt_cache_pending_key_hydrations: BTreeSet::new(),
            prompt_cache_candidate_refill_required: false,
            prompt_cache_applied_terminal_ids: HashSet::new(),
            prompt_cache_coalesced_event_count: 0,
            prompt_cache_full_hydration_count: 0,
            prompt_cache_bounded_key_hydration_count: 0,
            prompt_cache_baseline_at: None,
            prompt_cache_baseline_row_id: 0,
            prompt_cache_response_source: "memory",
            prompt_cache_reconcile_required: false,
            prompt_cache_pressure_deferred: false,
            latest_live_snapshot: None,
            calendar_anchor: None,
            continuity_reset_cursor: None,
            dashboard_materializer: None,
            dashboard_base_revision: cursor,
            dashboard_materialized_revision: None,
            snapshot_payload,
            snapshot_frame,
            snapshot_bytes: 32,
            replay_events,
            replay_bytes,
        }
    }

    fn test_replay_event(
        descriptor: &SubscriptionTopicDescriptor,
        schema_epoch: &str,
        cursor: u64,
        emitted_at: DateTime<Utc>,
        bytes: usize,
    ) -> ReplayableTopicEvent {
        let topic = SubscriptionTopic::from_descriptor(descriptor).expect("test topic");
        let frame = serialize_topic_frame(
            descriptor.clone(),
            topic.cache_key().expect("test topic key"),
            schema_epoch.to_string(),
            cursor,
            serde_json::to_vec(&json!({ "cursor": cursor })).expect("test payload"),
        )
        .expect("test frame");
        ReplayableTopicEvent {
            frame: Arc::new(frame),
            bytes,
            emitted_at,
        }
    }

    #[test]
    fn descriptor_round_trip_canonicalizes_sorted_params() {
        let descriptor = SubscriptionTopicDescriptor {
            topic: "stats.summary.current".to_string(),
            params: BTreeMap::from([
                ("timeZone".to_string(), "Asia/Shanghai".to_string()),
                ("window".to_string(), "current".to_string()),
                ("limit".to_string(), "20".to_string()),
            ]),
        };

        let topic = SubscriptionTopic::from_descriptor(&descriptor).expect("topic should parse");
        let canonical = topic.descriptor();

        assert_eq!(canonical.topic, "stats.summary.current");
        assert_eq!(
            canonical.params.get("window").map(String::as_str),
            Some("current")
        );
        assert_eq!(
            canonical.params.get("timeZone").map(String::as_str),
            Some("Asia/Shanghai")
        );
        assert_eq!(
            canonical.params.get("limit").map(String::as_str),
            Some("20")
        );
    }

    #[test]
    fn dashboard_working_conversations_descriptor_enforces_http_pagination_limits() {
        let descriptor =
            |page_size: &str, recent_invocation_limit: &str| SubscriptionTopicDescriptor {
                topic: "dashboard.working-conversations.current".to_string(),
                params: BTreeMap::from([
                    ("pageSize".to_string(), page_size.to_string()),
                    (
                        "recentInvocationLimit".to_string(),
                        recent_invocation_limit.to_string(),
                    ),
                ]),
            };

        assert!(SubscriptionTopic::from_descriptor(&descriptor("0", "4")).is_err());
        assert!(SubscriptionTopic::from_descriptor(&descriptor("101", "4")).is_err());
        assert!(SubscriptionTopic::from_descriptor(&descriptor("20", "3")).is_err());
        assert!(SubscriptionTopic::from_descriptor(&descriptor("20", "17")).is_err());

        let valid = descriptor("100", "16");
        assert_eq!(
            SubscriptionTopic::from_descriptor(&valid)
                .expect("valid working conversation descriptor")
                .descriptor(),
            valid
        );
    }

    #[tokio::test]
    async fn working_conversations_snapshot_builder_uses_one_transaction_snapshot() {
        let (state, temp_dir, _) = crate::tests::file_backed_test_state_with_busy_timeout(
            "working-conversations-snapshot",
            Duration::from_secs(1),
        )
        .await;
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&state.pool)
            .await
            .expect("enable concurrent reader and writer fixture");
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let insert = |invoke_id: &str, prompt_cache_key: &str| {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, payload, raw_response
                )
                VALUES (?1, ?2, 'proxy', 'success', ?3, '{}')
                "#,
            )
            .bind(invoke_id.to_string())
            .bind(occurred_at.clone())
            .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
        };
        insert("baseline-key", "baseline-key")
            .execute(&state.pool)
            .await
            .expect("insert baseline working conversation");

        let mut transaction = state
            .pool
            .begin()
            .await
            .expect("begin snapshot transaction");
        let baseline_row_id =
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                .fetch_one(transaction.as_mut())
                .await
                .expect("read snapshot row ceiling");
        insert("post-snapshot-key", "post-snapshot-key")
            .execute(&state.pool)
            .await
            .expect("insert post-snapshot working conversation");

        let response = build_prompt_cache_conversations_response_for_request_on_connection(
            state.as_ref(),
            PromptCacheConversationsRequest {
                selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
                detail_level: PromptCacheConversationDetailLevel::Full,
                recent_invocation_limit: Some(16),
                page_size: Some(20),
                cursor: None,
                snapshot_at: None,
                blocked_binding_filter: None,
            },
            transaction.as_mut(),
            Utc::now(),
            Some(baseline_row_id),
        )
        .await
        .expect("build transaction-pinned working response");

        let keys = response
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["baseline-key"]);
        let cursor = response.conversations[0]
            .cursor
            .as_deref()
            .expect("working conversation cursor");
        assert_eq!(
            decode_prompt_cache_conversation_cursor(cursor)
                .expect("decode working conversation cursor")
                .3,
            Some(baseline_row_id)
        );
        transaction
            .commit()
            .await
            .expect("commit snapshot transaction");
        state.pool.close().await;
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn working_conversations_paginated_snapshot_keeps_pre_snapshot_runtime_after_update() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let snapshot_at = Utc::now();
        let before_snapshot = format_naive(
            (snapshot_at - ChronoDuration::seconds(1))
                .with_timezone(&Shanghai)
                .naive_local(),
        );

        let mut running = dashboard_runtime_topology_live_record(&before_snapshot);
        running.id = 0;
        running.invoke_id = "working-runtime-pre-snapshot-update".to_string();
        running.status = Some("running".to_string());
        running.live_phase = Some("streaming".to_string());
        running.prompt_cache_key = Some("working-runtime-pre-snapshot-update-key".to_string());
        running.created_at = format_utc_iso_precise(snapshot_at - ChronoDuration::seconds(1));
        state.proxy_runtime_invocations.upsert(running.clone());

        let mut terminal_update = running;
        terminal_update.status = Some("success".to_string());
        terminal_update.live_phase = None;
        terminal_update.total_tokens = Some(23);
        terminal_update.created_at =
            format_utc_iso_precise(snapshot_at + ChronoDuration::minutes(1));
        state
            .proxy_runtime_invocations
            .upsert_terminal(terminal_update);

        let mut post_snapshot = dashboard_runtime_topology_live_record(&format_naive(
            (snapshot_at + ChronoDuration::minutes(1))
                .with_timezone(&Shanghai)
                .naive_local(),
        ));
        post_snapshot.id = 0;
        post_snapshot.invoke_id = "working-runtime-post-snapshot".to_string();
        post_snapshot.status = Some("success".to_string());
        post_snapshot.live_phase = None;
        post_snapshot.prompt_cache_key = Some("working-runtime-post-snapshot-key".to_string());
        post_snapshot.created_at = format_utc_iso_precise(snapshot_at + ChronoDuration::minutes(1));
        state.proxy_runtime_invocations.upsert(post_snapshot);

        let response = build_prompt_cache_conversations_response_for_request(
            state.as_ref(),
            PromptCacheConversationsRequest {
                selection: PromptCacheConversationSelection::ActivityWindowMinutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
                detail_level: PromptCacheConversationDetailLevel::Full,
                recent_invocation_limit: Some(16),
                page_size: Some(20),
                cursor: None,
                snapshot_at: Some(format_utc_iso_precise(snapshot_at)),
                blocked_binding_filter: None,
            },
        )
        .await
        .expect("build frozen working conversations page");

        assert_eq!(response.total_matched, Some(1));
        assert_eq!(response.conversations.len(), 1);
        let conversation = &response.conversations[0];
        assert_eq!(
            conversation.prompt_cache_key,
            "working-runtime-pre-snapshot-update-key"
        );
        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 23);
        assert_eq!(conversation.recent_invocations.len(), 1);
    }

    #[tokio::test]
    async fn working_conversations_initial_baseline_preserves_prebaseline_key_for_hydration() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "prebaseline-working-key".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("prebaseline-working-key".to_string());
        record.total_tokens = Some(37);
        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build prebaseline working delta")
            .expect("working delta");
        let topic_key = topic.cache_key().expect("working conversation topic key");
        state
            .subscription_hub
            .state
            .lock()
            .await
            .prompt_cache_prebaseline_records
            .entry(topic_key)
            .or_default()
            .insert(delta.identity.clone(), delta.clone());

        let cached = state
            .subscription_hub
            .refresh_topic(state.clone(), topic, false)
            .await
            .expect("initial baseline must defer rather than drop a runtime-only key");

        assert!(
            cached
                .prompt_cache_pending_records
                .contains_key(&delta.identity)
        );
        assert!(
            cached
                .prompt_cache_pending_key_hydrations
                .contains("prebaseline-working-key")
        );
        assert!(cached.prompt_cache_key_hydration_scheduled);
    }

    #[tokio::test]
    async fn working_conversations_initial_baseline_dedupes_runtime_overlay_terminal_replay() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "runtime-overlay-pending-terminal".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("runtime-overlay-pending-key".to_string());
        record.total_tokens = Some(37);
        record.cost = Some(0.75);
        state.proxy_runtime_invocations.upsert(record.clone());
        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build prebaseline runtime terminal delta")
            .expect("working runtime terminal delta");
        let topic_key = topic.cache_key().expect("working conversation topic key");
        state
            .subscription_hub
            .state
            .lock()
            .await
            .prompt_cache_prebaseline_records
            .entry(topic_key)
            .or_default()
            .insert(delta.identity.clone(), delta);

        let cached = state
            .subscription_hub
            .refresh_topic(state.clone(), topic, false)
            .await
            .expect("initial baseline must not double-count its runtime overlay");
        let DashboardTopicMaterializer::WorkingConversations {
            state: materializer,
        } = cached
            .dashboard_materializer
            .as_ref()
            .expect("typed working materializer")
        else {
            panic!("expected typed working conversations materializer");
        };
        let materializer = materializer
            .lock()
            .expect("working conversations materializer state lock");
        let conversation = materializer
            .response
            .conversations
            .iter()
            .find(|conversation| conversation.prompt_cache_key == "runtime-overlay-pending-key")
            .expect("runtime overlay conversation is present");
        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 37);
        assert!((conversation.total_cost - 0.75).abs() < f64::EPSILON);
        assert_eq!(conversation.last24h_requests.len(), 1);
        assert_eq!(conversation.recent_invocations.len(), 1);
    }

    #[tokio::test]
    async fn working_conversations_initial_baseline_dedupes_persisted_runtime_overlay() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "persisted-runtime-overlay-baseline".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("persisted-runtime-overlay-key".to_string());
        record.upstream_account_id = Some(77);
        record.upstream_account_name = Some("Persisted Overlay Account".to_string());
        record.total_tokens = Some(37);
        record.cost = Some(0.75);
        state.proxy_runtime_invocations.upsert(record.clone());
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, 'proxy', 'success', 37, 0.75, ?3, '{}')
            "#,
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .bind(
            json!({
                "promptCacheKey": "persisted-runtime-overlay-key",
                "upstreamAccountId": 77,
                "upstreamAccountName": "Persisted Overlay Account",
            })
            .to_string(),
        )
        .execute(&state.pool)
        .await
        .expect("persist terminal before runtime overlay acknowledgement");

        let cached = state
            .subscription_hub
            .refresh_topic(state.clone(), topic, false)
            .await
            .expect("initial baseline must dedupe a persisted runtime overlay");
        let DashboardTopicMaterializer::WorkingConversations {
            state: materializer,
        } = cached
            .dashboard_materializer
            .as_ref()
            .expect("typed working materializer")
        else {
            panic!("expected typed working conversations materializer");
        };
        let materializer = materializer
            .lock()
            .expect("working conversations materializer state lock");
        let conversation = materializer
            .response
            .conversations
            .iter()
            .find(|conversation| conversation.prompt_cache_key == "persisted-runtime-overlay-key")
            .expect("persisted runtime overlay conversation is present");
        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 37);
        assert!((conversation.total_cost - 0.75).abs() < f64::EPSILON);
        assert_eq!(conversation.last24h_requests.len(), 1);
        assert_eq!(conversation.recent_invocations.len(), 1);
        assert_eq!(conversation.upstream_accounts.len(), 1);
        assert_eq!(conversation.upstream_accounts[0].request_count, 1);
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_restores_reentered_key_contract() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register working conversations topic");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("build empty working conversations baseline");

        let now = Utc::now();
        // Bounded hydration reads current-hour P1 rows plus materialized prior-hour history.
        // Keep both fixture rows in the live hour so the contract does not depend on an
        // hour-rollup boundary while CI happens to run near the top of an hour.
        let occurred_at = format_naive(now.with_timezone(&Shanghai).naive_local());
        for (invoke_id, occurred_at, total_tokens) in [
            ("reentered-older", occurred_at.as_str(), 11_i64),
            ("reentered-newer", occurred_at.as_str(), 31_i64),
        ] {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                ) VALUES (?1, ?2, 'proxy', 'success', ?3, 0.25, ?4, '{}')
                "#,
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .bind(total_tokens)
            .bind(
                json!({
                    "promptCacheKey": "reentered-key",
                    "upstreamAccountId": 42,
                    "upstreamAccountName": "Hydrated Account",
                })
                .to_string(),
            )
            .execute(&state.pool)
            .await
            .expect("persist reentered working conversation history");
        }
        // The hydration snapshot is captured after these writes. If CI crosses an hour
        // boundary in between, account details read the materialized historical bucket
        // rather than the prior-hour P1 rows, just as production does after rollup.
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_upstream_account_hourly (
                bucket_start_epoch,
                source,
                prompt_cache_key,
                upstream_account_key,
                upstream_account_id,
                upstream_account_name,
                request_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                first_seen_at,
                last_seen_at,
                updated_at
            )
            VALUES (?1, 'proxy', 'reentered-key', 'id:42|name:Hydrated Account', 42,
                    'Hydrated Account', 2, 2, 0, 42, 0.5, ?2, ?2, datetime('now'))
            "#,
        )
        .bind(now.timestamp().div_euclid(3_600) * 3_600)
        .bind(&occurred_at)
        .execute(&state.pool)
        .await
        .expect("materialize reentered working conversation account history");
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_conversation_bindings (
                prompt_cache_key, binding_kind, group_name, upstream_account_id, created_at, updated_at
            ) VALUES ('reentered-key', 'group', 'Bounded Group', NULL, datetime('now'), datetime('now'))
            "#,
        )
        .execute(&state.pool)
        .await
        .expect("persist reentered key manual binding");

        let topic_key = topic.cache_key().expect("working conversation topic key");
        {
            let mut guard = state.subscription_hub.state.lock().await;
            let cached = guard
                .topics
                .get_mut(&topic_key)
                .expect("cached working conversations topic");
            cached
                .prompt_cache_pending_key_hydrations
                .insert("reentered-key".to_string());
            cached.prompt_cache_key_hydration_scheduled = true;
        }
        state
            .subscription_hub
            .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
            .await
            .expect("hydrate reentered key without a full working-window rebuild");

        let guard = state.subscription_hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("hydrated working conversations topic");
        let DashboardTopicMaterializer::WorkingConversations {
            state: materializer,
        } = cached
            .dashboard_materializer
            .as_ref()
            .expect("typed working materializer")
        else {
            panic!("expected typed working conversations materializer");
        };
        let materializer = materializer
            .lock()
            .expect("working conversations materializer state lock");
        let conversation = materializer
            .response
            .conversations
            .iter()
            .find(|conversation| conversation.prompt_cache_key == "reentered-key")
            .expect("bounded hydrate must restore the reentered key");
        assert_eq!(conversation.request_count, 2);
        assert_eq!(conversation.total_tokens, 42);
        assert_eq!(conversation.last24h_requests.len(), 2);
        assert_eq!(conversation.recent_invocations.len(), 2);
        assert_eq!(
            conversation
                .manual_binding
                .as_ref()
                .and_then(|binding| binding.group_name.as_deref()),
            Some("Bounded Group"),
            "key hydration must restore manual binding metadata",
        );
        assert_eq!(
            conversation.upstream_accounts[0].upstream_account_id,
            Some(42),
            "key hydration must retain account metadata rather than synthesizing a delta-only row",
        );
        assert_eq!(cached.prompt_cache_full_hydration_count, 1);
        assert_eq!(cached.prompt_cache_bounded_key_hydration_count, 1);
        assert!(cached.prompt_cache_pending_key_hydrations.is_empty());
        assert!(!cached.prompt_cache_reconcile_required);
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_refills_active_page_candidates() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 1,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register working conversations topic");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("build empty working conversations baseline");

        let now = Utc::now();
        for (invoke_id, prompt_cache_key, occurred_at) in [
            (
                "candidate-older",
                "candidate-older",
                format_naive(
                    (now - ChronoDuration::minutes(2))
                        .with_timezone(&Shanghai)
                        .naive_local(),
                ),
            ),
            (
                "candidate-newer",
                "candidate-newer",
                format_naive(
                    (now - ChronoDuration::minutes(1))
                        .with_timezone(&Shanghai)
                        .naive_local(),
                ),
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, payload, raw_response
                ) VALUES (?1, ?2, 'proxy', 'success', 1, ?3, '{}')
                "#,
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
            .execute(&state.pool)
            .await
            .expect("persist candidate conversation");
        }

        let topic_key = topic.cache_key().expect("working conversation topic key");
        {
            let mut guard = state.subscription_hub.state.lock().await;
            let cached = guard
                .topics
                .get_mut(&topic_key)
                .expect("cached working conversations topic");
            cached.prompt_cache_candidate_refill_required = true;
            cached.prompt_cache_key_hydration_scheduled = true;
        }
        state
            .subscription_hub
            .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
            .await
            .expect("refill active page candidates with bounded hydration");

        let guard = state.subscription_hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("hydrated working conversations topic");
        let DashboardTopicMaterializer::WorkingConversations {
            state: materializer,
        } = cached
            .dashboard_materializer
            .as_ref()
            .expect("typed working materializer")
        else {
            panic!("expected typed working conversations materializer");
        };
        let materializer = materializer
            .lock()
            .expect("working conversations materializer state lock");
        assert_eq!(materializer.response.total_matched, Some(2));
        assert!(materializer.response.has_more);
        assert_eq!(materializer.response.conversations.len(), 1);
        assert_eq!(
            materializer.response.conversations[0].prompt_cache_key, "candidate-newer",
            "candidate refill must hydrate only the selected active page in sort order",
        );
        assert!(!cached.prompt_cache_candidate_refill_required);
        assert_eq!(cached.prompt_cache_bounded_key_hydration_count, 1);
        assert_eq!(cached.prompt_cache_full_hydration_count, 1);
        assert!(!cached.prompt_cache_reconcile_required);
    }

    #[tokio::test]
    async fn working_conversations_hydration_keeps_an_unresolved_eligible_delta_for_recovery() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let topic = SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size: 20,
            recent_invocation_limit: 16,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        };
        let _lease = state
            .subscription_hub
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register working conversations topic");
        state
            .subscription_hub
            .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("build empty working conversations baseline");

        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.invoke_id = "unresolved-working-hydration".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("unresolved-working-hydration".to_string());
        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build unresolved working delta")
            .expect("working delta");
        let topic_key = topic.cache_key().expect("working conversation topic key");
        {
            let mut guard = state.subscription_hub.state.lock().await;
            let cached = guard
                .topics
                .get_mut(&topic_key)
                .expect("cached working conversations topic");
            cached
                .prompt_cache_pending_records
                .insert(delta.identity.clone(), delta.clone());
            cached
                .prompt_cache_pending_key_hydrations
                .insert("unresolved-working-hydration".to_string());
            cached.prompt_cache_key_hydration_scheduled = true;
        }

        state
            .subscription_hub
            .hydrate_dashboard_working_conversation_keys(state.clone(), &topic)
            .await
            .expect("attempt bounded key hydration");

        let guard = state.subscription_hub.state.lock().await;
        let cached = guard
            .topics
            .get(&topic_key)
            .expect("cached working conversations topic");
        assert!(
            cached.dirty,
            "an eligible delta absent from the bounded hydration result must enter recovery"
        );
        assert!(cached.prompt_cache_reconcile_required);
        assert!(
            cached
                .prompt_cache_pending_records
                .contains_key(&delta.identity),
            "recovery must retain the unresolved eligible delta instead of silently dropping it"
        );
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_keeps_runtime_terminal_details() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_end = Utc::now();
        let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "runtime-only-working-hydration".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("runtime-only-working-hydration".to_string());
        record.upstream_account_id = Some(77);
        record.upstream_account_name = Some("Runtime Hydration Account".to_string());
        record.total_tokens = Some(37);
        record.cost = Some(0.75);
        state.proxy_runtime_invocations.upsert(record);

        let source_scope = resolve_default_source_scope(&state.pool)
            .await
            .expect("resolve default source scope");
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let conversation = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            "runtime-only-working-hydration",
            range_end,
            &range_start_bound,
            16,
            None,
        )
        .await
        .expect("hydrate runtime-only working conversation")
        .expect("runtime-only terminal key is selected");

        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 37);
        assert_eq!(conversation.last24h_requests.len(), 1);
        assert_eq!(conversation.last24h_requests[0].request_tokens, 37);
        assert_eq!(conversation.upstream_accounts.len(), 1);
        assert_eq!(
            conversation.upstream_accounts[0].upstream_account_id,
            Some(77)
        );
        assert_eq!(conversation.upstream_accounts[0].request_count, 1);
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_dedupes_persisted_runtime_overlay() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_end = Utc::now();
        let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "persisted-runtime-overlay-hydration".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("persisted-runtime-overlay-hydration-key".to_string());
        record.upstream_account_id = Some(78);
        record.upstream_account_name = Some("Persisted Hydration Account".to_string());
        record.total_tokens = Some(41);
        record.cost = Some(0.5);
        state.proxy_runtime_invocations.upsert(record.clone());
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, 'proxy', 'success', 41, 0.5, ?3, '{}')
            "#,
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .bind(
            json!({
                "promptCacheKey": "persisted-runtime-overlay-hydration-key",
                "upstreamAccountId": 78,
                "upstreamAccountName": "Persisted Hydration Account",
            })
            .to_string(),
        )
        .execute(&state.pool)
        .await
        .expect("persist terminal before bounded hydration acknowledgement");

        let source_scope = resolve_default_source_scope(&state.pool)
            .await
            .expect("resolve default source scope");
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let conversation = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            "persisted-runtime-overlay-hydration-key",
            range_end,
            &range_start_bound,
            16,
            None,
        )
        .await
        .expect("hydrate persisted runtime overlay working conversation")
        .expect("persisted runtime overlay key is selected");

        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 41);
        assert!((conversation.total_cost - 0.5).abs() < f64::EPSILON);
        assert_eq!(conversation.last24h_requests.len(), 1);
        assert_eq!(conversation.recent_invocations.len(), 1);
        assert_eq!(conversation.upstream_accounts.len(), 1);
        assert_eq!(conversation.upstream_accounts[0].request_count, 1);
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_keeps_runtime_overlay_after_snapshot_boundary()
    {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_end = Utc::now();
        let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "post-snapshot-runtime-overlay-hydration".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("post-snapshot-runtime-overlay-hydration-key".to_string());
        record.upstream_account_id = Some(79);
        record.upstream_account_name = Some("Post Snapshot Hydration Account".to_string());
        record.total_tokens = Some(43);
        record.cost = Some(0.6);
        state.proxy_runtime_invocations.upsert(record.clone());

        // Simulate P1 committing after the bounded hydrate's durable visibility boundary. The
        // durable row must not remove the runtime terminal until a matching snapshot can see it.
        let created_after_snapshot = format_utc_iso_precise(range_end + ChronoDuration::minutes(5));
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload,
                raw_response, created_at
            ) VALUES (?1, ?2, 'proxy', 'success', 43, 0.6, ?3, '{}', ?4)
            "#,
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .bind(
            json!({
                "promptCacheKey": "post-snapshot-runtime-overlay-hydration-key",
                "upstreamAccountId": 79,
                "upstreamAccountName": "Post Snapshot Hydration Account",
            })
            .to_string(),
        )
        .bind(created_after_snapshot)
        .execute(&state.pool)
        .await
        .expect("persist terminal beyond bounded hydration snapshot");

        let source_scope = resolve_default_source_scope(&state.pool)
            .await
            .expect("resolve default source scope");
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let conversation = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            "post-snapshot-runtime-overlay-hydration-key",
            range_end,
            &range_start_bound,
            16,
            None,
        )
        .await
        .expect("hydrate post-snapshot persisted runtime overlay")
        .expect("runtime overlay key is selected");

        assert_eq!(conversation.request_count, 1);
        assert_eq!(conversation.total_tokens, 43);
        assert!((conversation.total_cost - 0.6).abs() < f64::EPSILON);
        assert_eq!(conversation.last24h_requests.len(), 1);
        assert_eq!(conversation.recent_invocations.len(), 1);
        assert_eq!(conversation.upstream_accounts.len(), 1);
        assert_eq!(conversation.upstream_accounts[0].request_count, 1);
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_prefers_p1_lifecycle_over_stale_working_set() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_end = Utc::now();
        let old_occurred_at = format_naive(
            (range_end - ChronoDuration::minutes(2))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let newest_occurred_at = format_naive(
            (range_end - ChronoDuration::minutes(1))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let prompt_cache_key = "p1-before-p2-working-hydration";
        for (invoke_id, occurred_at, total_tokens) in [
            ("p1-before-p2-old", old_occurred_at.as_str(), 11_i64),
            ("p1-before-p2-new", newest_occurred_at.as_str(), 31_i64),
        ] {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                ) VALUES (?1, ?2, 'proxy', 'success', ?3, 0.25, ?4, '{}')
                "#,
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .bind(total_tokens)
            .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
            .execute(&state.pool)
            .await
            .expect("persist terminal before P2 working-set refresh");
        }
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_working_set_live (
                prompt_cache_key, source_scope_all, source_scope_proxy_only,
                created_at, last_activity_at, last_terminal_at, sort_anchor_at,
                request_count, total_tokens, total_cost, updated_at
            ) VALUES (?1, 1, 1, ?2, ?2, ?2, ?2, 1, 11, 0.25, datetime('now'))
            ON CONFLICT(prompt_cache_key) DO UPDATE SET
                source_scope_all = excluded.source_scope_all,
                source_scope_proxy_only = excluded.source_scope_proxy_only,
                created_at = excluded.created_at,
                last_activity_at = excluded.last_activity_at,
                last_terminal_at = excluded.last_terminal_at,
                last_in_flight_at = NULL,
                sort_anchor_at = excluded.sort_anchor_at,
                request_count = excluded.request_count,
                total_tokens = excluded.total_tokens,
                total_cost = excluded.total_cost,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(prompt_cache_key)
        .bind(&old_occurred_at)
        .execute(&state.pool)
        .await
        .expect("seed stale P2 working-set row");

        let source_scope = resolve_default_source_scope(&state.pool)
            .await
            .expect("resolve default source scope");
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let conversation = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            prompt_cache_key,
            range_end,
            &range_start_bound,
            16,
            None,
        )
        .await
        .expect("hydrate a key with P1 ahead of P2")
        .expect("P1 lifecycle remains selected");

        assert_eq!(conversation.request_count, 2);
        assert_eq!(conversation.total_tokens, 42);
        assert_eq!(conversation.recent_invocations.len(), 2);
        assert_eq!(
            conversation.last_terminal_at.as_deref(),
            Some(newest_occurred_at.as_str())
        );
        assert_eq!(conversation.last_activity_at, newest_occurred_at);
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_keeps_transient_runtime_records_after_filtering_persisted_terminal()
     {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_end = Utc::now();
        let occurred_at = format_naive(range_end.with_timezone(&Shanghai).naive_local());
        let prompt_cache_key = "bounded-hydration-overlay-filter-key";
        let mut persisted_terminal = dashboard_runtime_topology_live_record(&occurred_at);
        persisted_terminal.id = 0;
        persisted_terminal.invoke_id = "bounded-hydration-persisted-terminal".to_string();
        persisted_terminal.status = Some("success".to_string());
        persisted_terminal.live_phase = None;
        persisted_terminal.prompt_cache_key = Some(prompt_cache_key.to_string());
        persisted_terminal.upstream_account_id = Some(79);
        persisted_terminal.upstream_account_name = Some("Bounded Filter Account".to_string());
        persisted_terminal.total_tokens = Some(41);
        persisted_terminal.cost = Some(0.5);
        state
            .proxy_runtime_invocations
            .upsert(persisted_terminal.clone());
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, ?2, 'proxy', 'success', 41, 0.5, ?3, '{}')
            "#,
        )
        .bind(&persisted_terminal.invoke_id)
        .bind(&persisted_terminal.occurred_at)
        .bind(
            json!({
                "promptCacheKey": prompt_cache_key,
                "upstreamAccountId": 79,
                "upstreamAccountName": "Bounded Filter Account",
            })
            .to_string(),
        )
        .execute(&state.pool)
        .await
        .expect("persist terminal before bounded hydration acknowledgement");

        let mut runtime_terminal = persisted_terminal.clone();
        runtime_terminal.invoke_id = "bounded-hydration-runtime-terminal".to_string();
        runtime_terminal.total_tokens = Some(37);
        runtime_terminal.cost = Some(0.75);
        state
            .proxy_runtime_invocations
            .upsert(runtime_terminal.clone());

        let mut in_flight = persisted_terminal.clone();
        in_flight.invoke_id = "bounded-hydration-runtime-in-flight".to_string();
        in_flight.status = Some("running".to_string());
        in_flight.live_phase = Some("requesting".to_string());
        in_flight.total_tokens = None;
        in_flight.cost = None;
        state.proxy_runtime_invocations.upsert(in_flight.clone());

        let source_scope = resolve_default_source_scope(&state.pool)
            .await
            .expect("resolve default source scope");
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let conversation = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            prompt_cache_key,
            range_end,
            &range_start_bound,
            16,
            None,
        )
        .await
        .expect("hydrate mixed runtime overlay working conversation")
        .expect("mixed runtime overlay key is selected");

        assert_eq!(conversation.request_count, 3);
        assert_eq!(conversation.total_tokens, 78);
        assert!((conversation.total_cost - 1.25).abs() < f64::EPSILON);
        assert_eq!(conversation.recent_invocations.len(), 3);
        assert!(
            conversation
                .recent_invocations
                .iter()
                .any(|preview| preview.invoke_id == runtime_terminal.invoke_id)
        );
        assert!(
            conversation
                .recent_invocations
                .iter()
                .any(|preview| preview.invoke_id == in_flight.invoke_id)
        );
        assert!(conversation.last_in_flight_at.is_some());
    }

    #[tokio::test]
    async fn working_conversations_bounded_hydration_keeps_old_in_flight_off_the_chart() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_end = Utc::now();
        let occurred_at = format_naive(
            (range_end - ChronoDuration::hours(PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS + 1))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let mut record = dashboard_runtime_topology_live_record(&occurred_at);
        record.id = 0;
        record.invoke_id = "old-runtime-working-hydration".to_string();
        record.prompt_cache_key = Some("old-runtime-working-hydration".to_string());
        state.proxy_runtime_invocations.upsert(record);

        let source_scope = resolve_default_source_scope(&state.pool)
            .await
            .expect("resolve default source scope");
        let range_start_bound = db_occurred_at_lower_bound(
            range_end
                - ChronoDuration::minutes(
                    SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                ),
        );
        let conversation = hydrate_working_prompt_cache_conversation_for_key(
            state.as_ref(),
            source_scope,
            "old-runtime-working-hydration",
            range_end,
            &range_start_bound,
            16,
            None,
        )
        .await
        .expect("hydrate old in-flight working conversation")
        .expect("old in-flight key remains selected");

        assert!(conversation.last_in_flight_at.is_some());
        assert!(conversation.last24h_requests.is_empty());
    }

    #[test]
    fn working_conversations_hydration_detects_pending_delta_changes_after_snapshot() {
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let delta = |invoke_id: &str| {
            let mut record = dashboard_runtime_topology_live_record(&occurred_at);
            record.id = 0;
            record.invoke_id = invoke_id.to_string();
            record.status = Some("success".to_string());
            record.live_phase = None;
            record.prompt_cache_key = Some("hydration-race-key".to_string());
            PromptCacheTopicDelta::from_record(&record)
                .expect("build hydration race delta")
                .expect("typed hydration race delta")
        };
        let initial = delta("hydration-race-initial");
        let before = BTreeMap::from([(initial.identity.clone(), initial.clone())]);
        let hydration_keys = vec!["hydration-race-key".to_string()];

        assert!(
            prompt_cache_hydration_changed_pending_keys(&before, &before, &hydration_keys)
                .is_empty(),
            "a stable pending snapshot must not force recovery",
        );

        let later = delta("hydration-race-later");
        let mut with_later_identity = before.clone();
        with_later_identity.insert(later.identity.clone(), later);
        assert_eq!(
            prompt_cache_hydration_changed_pending_keys(
                &before,
                &with_later_identity,
                &hydration_keys,
            ),
            BTreeSet::from(["hydration-race-key".to_string()]),
            "a delta added after the hydrate snapshot must retry only its bounded key",
        );

        let mut transitioned = initial.clone();
        transitioned.status = "running".to_string();
        transitioned.is_terminal = false;
        transitioned.preview.as_mut().expect("typed preview").status = "running".to_string();
        let with_replaced_identity =
            BTreeMap::from([(transitioned.identity.clone(), transitioned)]);
        assert_eq!(
            prompt_cache_hydration_changed_pending_keys(
                &before,
                &with_replaced_identity,
                &hydration_keys,
            ),
            BTreeSet::from(["hydration-race-key".to_string()]),
            "a same-identity state transition after the hydrate snapshot must retry its key",
        );
    }

    #[test]
    fn dashboard_network_recent_topic_uses_empty_descriptor_and_stable_schema_epoch() {
        let descriptor = SubscriptionTopicDescriptor {
            topic: "dashboard.network-recent.current".to_string(),
            params: BTreeMap::new(),
        };

        let topic =
            SubscriptionTopic::from_descriptor(&descriptor).expect("recent topic should parse");

        assert_eq!(topic.descriptor(), descriptor);
        assert_eq!(topic.name(), "dashboard.network-recent.current");
        assert_eq!(topic.schema_epoch(), "dashboard.network-recent.current/v1");
        assert!(
            topic.is_affected_by_runtime_mutation(&RuntimeMutation::Invocation(
                RuntimeInvocationMutation {
                    identity: RuntimeInvocationIdentity::new("network", "2026-07-20 00:00:00"),
                    kind: RuntimeMutationKind::RuntimeUpsert,
                    row_id: None,
                    is_terminal: false,
                    prompt_cache_key: None,
                    sticky_key: None,
                    upstream_account_id: None,
                }
            ))
        );
        assert!(
            !topic.is_affected_by(&BroadcastPayload::DashboardActivityLive {
                snapshot: Box::new(DashboardActivityLiveSnapshot {
                    revision: 1,
                    generated_at: "2026-07-20T00:00:00.000Z".to_string(),
                    in_progress_invocation_count: 0,
                    in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                    retry_invocation_count: 0,
                    in_progress_wait_sum_ms: 0.0,
                    in_progress_wait_sample_count: 0,
                    network_live_bucket: None,
                    network_realtime_rate: None,
                    accounts: Vec::new(),
                }),
            })
        );
    }

    #[test]
    fn model_routing_live_sse_topic_is_bounded_and_refreshes_only_on_route_updates() {
        let descriptor = SubscriptionTopicDescriptor {
            topic: "pool.model-routing-live".to_string(),
            params: BTreeMap::from([
                ("window".to_string(), "1h".to_string()),
                ("model".to_string(), "gpt-5.5".to_string()),
                ("state".to_string(), "cooling_down".to_string()),
                ("limit".to_string(), "100".to_string()),
            ]),
        };
        let topic = SubscriptionTopic::from_descriptor(&descriptor)
            .expect("model routing topic descriptor should parse");

        assert_eq!(topic.descriptor(), descriptor);
        assert_eq!(topic.class(), SubscriptionTopicClass::BoundedColdHydrate);
        assert_eq!(topic.schema_epoch(), "pool.model-routing-live/v1");
        assert_eq!(
            topic.runtime_topic_dependencies(),
            vec![RuntimeTopicDependency::ModelRouting]
        );
        assert!(topic.is_affected_by_runtime_mutation(&RuntimeMutation::ModelRoutingChanged));
        assert!(
            !topic.is_affected_by_runtime_mutation(&RuntimeMutation::AttemptChanged {
                invoke_id: "unrelated".to_string(),
            })
        );
        assert!(
            SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
                topic: "pool.model-routing-live".to_string(),
                params: BTreeMap::from([("window".to_string(), "48h".to_string())]),
            })
            .is_err()
        );
    }

    #[test]
    fn conversation_detail_topics_require_an_unambiguous_scope() {
        let prompt_cache_descriptor = SubscriptionTopicDescriptor {
            topic: "invocation-history.window".to_string(),
            params: BTreeMap::from([("promptCacheKey".to_string(), "pck-1".to_string())]),
        };
        let sticky_descriptor = SubscriptionTopicDescriptor {
            topic: "prompt-cache.conversation-operations.window".to_string(),
            params: BTreeMap::from([
                ("stickyKey".to_string(), "sticky-1".to_string()),
                ("upstreamAccountId".to_string(), "42".to_string()),
                ("infoType".to_string(), "routing".to_string()),
            ]),
        };

        let prompt_cache_topic = SubscriptionTopic::from_descriptor(&prompt_cache_descriptor)
            .expect("prompt cache scope should parse");
        let sticky_topic = SubscriptionTopic::from_descriptor(&sticky_descriptor)
            .expect("sticky scope should parse");

        assert_eq!(prompt_cache_topic.descriptor(), prompt_cache_descriptor);
        assert_eq!(sticky_topic.descriptor(), sticky_descriptor);
        assert!(
            SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
                topic: "invocation-history.overview".to_string(),
                params: BTreeMap::new(),
            })
            .is_err()
        );
        assert!(
            SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
                topic: "invocation-history.overview".to_string(),
                params: BTreeMap::from([
                    ("promptCacheKey".to_string(), "pck-1".to_string()),
                    ("stickyKey".to_string(), "sticky-1".to_string()),
                    ("upstreamAccountId".to_string(), "42".to_string()),
                ]),
            })
            .is_err()
        );
    }

    #[test]
    fn typed_runtime_binding_events_only_refresh_binding_and_operations_topics() {
        let scope_params = BTreeMap::from([("promptCacheKey".to_string(), "pck-1".to_string())]);
        let calls = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "invocation-history.window".to_string(),
            params: scope_params.clone(),
        })
        .expect("calls topic should parse");
        let overview = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "invocation-history.overview".to_string(),
            params: scope_params.clone(),
        })
        .expect("overview topic should parse");
        let binding = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "prompt-cache.conversation-binding.current".to_string(),
            params: scope_params.clone(),
        })
        .expect("binding topic should parse");
        let operations = SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "prompt-cache.conversation-operations.window".to_string(),
            params: scope_params,
        })
        .expect("operations topic should parse");
        let event = RuntimeMutation::PromptCacheBindingChanged {
            prompt_cache_key: "pck-1".to_string(),
        };

        assert!(!calls.is_affected_by_runtime_mutation(&event));
        assert!(!overview.is_affected_by_runtime_mutation(&event));
        assert!(binding.is_affected_by_runtime_mutation(&event));
        assert!(operations.is_affected_by_runtime_mutation(&event));
        assert!(!binding.is_affected_by_runtime_mutation(
            &RuntimeMutation::PromptCacheBindingChanged {
                prompt_cache_key: "pck-2".to_string(),
            }
        ));
        assert!(
            !binding.is_affected_by_runtime_mutation(&RuntimeMutation::AttemptChanged {
                invoke_id: "other".to_string(),
            })
        );
        assert!(
            !operations.is_affected_by_runtime_mutation(&RuntimeMutation::AttemptChanged {
                invoke_id: "other".to_string(),
            })
        );
    }

    #[test]
    fn sticky_route_changes_refresh_only_the_previous_and_current_history_scopes() {
        let topic_for = |upstream_account_id| SubscriptionTopic::InvocationHistoryWindow {
            scope: ConversationSubscriptionScope::StickyKey {
                sticky_key: "sticky-1".to_string(),
                upstream_account_id,
            },
        };
        let prompt_cache_topic = SubscriptionTopic::InvocationHistoryOverview {
            scope: ConversationSubscriptionScope::PromptCacheKey("sticky-1".to_string()),
        };
        let event = RuntimeMutation::StickyRouteChanged {
            sticky_key: "sticky-1".to_string(),
            previous_upstream_account_id: 41,
            upstream_account_id: 42,
        };

        assert!(topic_for(41).is_affected_by_runtime_mutation(&event));
        assert!(topic_for(42).is_affected_by_runtime_mutation(&event));
        assert!(!topic_for(43).is_affected_by_runtime_mutation(&event));
        assert!(prompt_cache_topic.is_affected_by_runtime_mutation(&event));
    }

    #[tokio::test]
    async fn conversation_overview_refresh_marks_events_arriving_during_rebuild_for_rerun() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = SubscriptionHub::new();
        let topic = SubscriptionTopic::InvocationHistoryOverview {
            scope: ConversationSubscriptionScope::PromptCacheKey("pck-1".to_string()),
        };
        let topic_key = topic.cache_key().expect("conversation overview topic key");

        {
            let mut guard = hub.state.lock().await;
            let mut cached = seeded_cached_topic(topic.clone(), &[], Utc::now());
            cached.conversation_overview_refresh_scheduled = true;
            cached.conversation_overview_refresh_in_flight = true;
            guard.topics.insert(topic_key.clone(), cached);
            guard.active_subscribers.insert(topic_key.clone(), 1);
        }

        hub.schedule_conversation_overview_topic_refresh(state, topic)
            .await
            .expect("in-flight conversation overview refresh should accept a pending event");

        let guard = hub.state.lock().await;
        assert!(
            guard
                .topics
                .get(&topic_key)
                .is_some_and(|cached| cached.conversation_overview_refresh_pending)
        );
    }

    #[tokio::test]
    async fn conversation_overview_includes_runtime_records_in_chart_samples() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let prompt_cache_key = "runtime-overview-pck";
        let occurred_at = crate::proxy::shanghai_now_string();
        let runtime_record = crate::proxy::build_admitted_proxy_capture_runtime_snapshot(
            "runtime-overview-invoke",
            &occurred_at,
            ProxyCaptureTarget::Responses,
            None,
            None,
            Some(prompt_cache_key),
        );
        state
            .proxy_runtime_invocations
            .upsert(crate::proxy::api_invocation_from_runtime_record(
                &runtime_record,
            ));

        let topic = SubscriptionTopic::InvocationHistoryOverview {
            scope: ConversationSubscriptionScope::PromptCacheKey(prompt_cache_key.to_string()),
        };
        let payload = topic
            .build_payload(state)
            .await
            .expect("conversation overview payload should build");

        assert_eq!(
            payload
                .pointer("/summary/totalCount")
                .and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(payload.get("chartTotal").and_then(Value::as_i64), Some(1));
        assert_eq!(
            payload
                .get("records")
                .and_then(Value::as_array)
                .and_then(|records| records.first())
                .and_then(|record| record.get("invokeId"))
                .and_then(Value::as_str),
            Some("runtime-overview-invoke")
        );
    }

    #[tokio::test]
    async fn conversation_overview_keeps_non_divisor_page_limits_contiguous() {
        let mut state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        Arc::get_mut(&mut state)
            .expect("overview test state should not have external owners")
            .config
            .list_limit_max = 37;
        let prompt_cache_key = "overview-page-limit-pck";
        for index in 0..75 {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, payload, raw_response
                )
                VALUES (?1, ?2, 'proxy', 'success', ?3, '{}')
                "#,
            )
            .bind(format!("overview-page-limit-{index:03}"))
            .bind(format!("2026-03-02 12:{:02}:{:02}", index / 60, index % 60))
            .bind(format!(r#"{{"promptCacheKey":"{prompt_cache_key}"}}"#))
            .execute(&state.pool)
            .await
            .expect("insert overview pagination seed row");
        }

        let topic = SubscriptionTopic::InvocationHistoryOverview {
            scope: ConversationSubscriptionScope::PromptCacheKey(prompt_cache_key.to_string()),
        };
        let payload = topic
            .build_payload(state)
            .await
            .expect("conversation overview payload should build");
        let records = payload
            .get("records")
            .and_then(Value::as_array)
            .expect("overview records should serialize");
        let mut invoke_ids = records
            .iter()
            .filter_map(|record| record.get("invokeId").and_then(Value::as_str))
            .collect::<Vec<_>>();
        invoke_ids.sort_unstable();
        invoke_ids.dedup();

        assert_eq!(payload.get("chartTotal").and_then(Value::as_i64), Some(75));
        assert_eq!(records.len(), 75);
        assert_eq!(invoke_ids.len(), 75);
    }

    #[test]
    fn dashboard_network_topics_use_projection_cadence_not_subscription_push_tasks() {
        let topics = [
            SubscriptionTopic::DashboardNetworkTimeseriesWindow {
                range: "today".to_string(),
                time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
                upstream_account_id: None,
            },
            SubscriptionTopic::DashboardNetworkRecentCurrent,
        ];

        for topic in topics {
            assert!(topic.uses_dashboard_network_live_snapshot());
            assert!(
                !topic.uses_server_push_cadence(RuntimeProjectionMode::Auto),
                "{} must be driven by the shared network projection cadence",
                topic.name()
            );
        }
        assert!(
            SubscriptionTopic::DashboardNetworkRecentCurrent
                .uses_server_push_cadence(RuntimeProjectionMode::Legacy)
        );
    }

    #[tokio::test]
    async fn dashboard_network_recent_legacy_push_cadence_emits_live_payload() {
        let state = crate::tests::test_state_with_openai_base_and_runtime_projection_mode(
            Url::parse("http://127.0.0.1:9").unwrap(),
            RuntimeProjectionMode::Legacy,
        )
        .await;
        let topic = SubscriptionTopic::DashboardNetworkRecentCurrent;
        let topic_key = topic.cache_key().expect("legacy recent topic key");
        let descriptor = topic.descriptor();
        let mut receiver = state.subscription_hub.subscribe();
        assert!(!topic.uses_server_push_cadence(RuntimeProjectionMode::Auto));
        assert!(topic.uses_server_push_cadence(RuntimeProjectionMode::Legacy));

        let _response = topic_sse_stream(
            State(state.clone()),
            Query(SubscriptionStreamQuery {
                topics: Some(
                    serde_json::to_string(std::slice::from_ref(&descriptor))
                        .expect("serialize legacy recent topic"),
                ),
                resume: None,
                attempt: Some(1),
                reason: Some("legacy-network-cadence-test".to_string()),
            }),
        )
        .await
        .expect("open legacy recent network SSE stream");
        {
            let guard = state.subscription_hub.state.lock().await;
            assert_eq!(
                guard.server_push_subscribers.get(&topic_key).copied(),
                Some(1),
                "legacy SSE entrypoint must retain the recent cadence owner",
            );
            assert!(guard.server_push_tasks.contains(&topic_key));
        }

        tokio::time::sleep(DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL * 2).await;
        state.dashboard_network_speed_cache.record_request_bytes(
            "legacy-recent-network-cadence",
            &crate::proxy::shanghai_now_string(),
            None,
            Some("api.openai.com"),
            128,
            Utc::now(),
        );

        let dispatch = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("legacy recent network push should be emitted")
            .expect("legacy recent network dispatch");

        assert_eq!(dispatch.frame.descriptor, descriptor);
        assert_eq!(
            dispatch.frame.schema_epoch,
            "dashboard.network-recent.current/v1"
        );
        let dispatch_payload = dispatch.frame.payload_value();
        assert_eq!(
            dispatch_payload
                .get("windowSeconds")
                .and_then(Value::as_i64),
            Some(300)
        );
        assert_eq!(
            dispatch_payload
                .get("sampleSeconds")
                .and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            dispatch_payload
                .get("points")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(300)
        );
    }

    #[test]
    fn prune_replay_window_enforces_event_cap() {
        let mut events = VecDeque::new();
        let mut total_bytes = 0usize;
        let topic = summary_topic();
        let descriptor = topic.descriptor();
        let schema_epoch = topic.schema_epoch();
        for index in 0..(SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC + 8) {
            events.push_back(test_replay_event(
                &descriptor,
                &schema_epoch,
                index as u64 + 1,
                Utc::now(),
                32,
            ));
            total_bytes += 32;
        }

        prune_replay_window(&mut events, &mut total_bytes);

        assert!(events.len() <= SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC);
    }

    #[test]
    fn serialized_topic_frame_reuses_shared_chunks_for_each_delivery_kind() {
        let topic = summary_topic();
        let descriptor = topic.descriptor();
        let payload = serde_json::to_vec(&json!({ "value": 42 })).expect("payload");
        let frame = serialize_topic_frame(
            descriptor,
            topic.cache_key().expect("topic key"),
            topic.schema_epoch(),
            7,
            payload,
        )
        .expect("serialized frame");

        for (kind, expected_kind) in [
            (TopicFrameKind::Snapshot, "snapshot"),
            (TopicFrameKind::Replay, "replay"),
            (TopicFrameKind::Live, "live"),
        ] {
            let chunks = frame.event_chunks(kind);
            assert_eq!(chunks[1].as_ptr(), frame.envelope_metadata_bytes.as_ptr());
            assert_eq!(chunks[2].as_ptr(), frame.payload_bytes.as_ptr());
            let wire = chunks.concat();
            let envelope: Value = serde_json::from_slice(&wire[6..wire.len() - 2])
                .expect("SSE data contains a JSON envelope");
            assert_eq!(envelope["type"], expected_kind);
            assert_eq!(envelope["cursor"], 7);
            assert_eq!(envelope["payload"]["value"], 42);
        }
        assert_eq!(
            frame.retained_bytes(),
            frame.envelope_metadata_bytes.len() + frame.payload_bytes.len()
        );
    }

    #[test]
    fn unchanged_refresh_clears_scheduled_flag_for_the_next_terminal_event() {
        let topic = summary_topic();
        let mut cached = seeded_cached_topic(topic, &[], Utc::now());
        cached.refresh_scheduled = true;
        cached.prompt_cache_reconcile_required = true;
        cached.prompt_cache_pressure_deferred = true;
        cached
            .prompt_cache_pending_key_hydrations
            .insert("pending-key".to_string());
        cached.prompt_cache_candidate_refill_required = true;
        cached.prompt_cache_key_hydration_scheduled = true;
        let payload = serde_json::to_vec(&cached.snapshot_payload).expect("cached payload");

        let reused = reuse_unchanged_cached_topic(&mut cached, &payload);

        assert!(reused.is_some());
        assert!(!cached.refresh_scheduled);
        assert!(!cached.prompt_cache_reconcile_required);
        assert!(!cached.prompt_cache_pressure_deferred);
        assert!(cached.prompt_cache_pending_key_hydrations.is_empty());
        assert!(!cached.prompt_cache_candidate_refill_required);
        assert!(!cached.prompt_cache_key_hydration_scheduled);
        cached.refresh_scheduled = true;
        assert!(
            cached.refresh_scheduled,
            "a later terminal event can schedule again"
        );
    }

    #[test]
    fn dirty_unchanged_refresh_clears_dirty_without_advancing_cursor() {
        let topic = summary_topic();
        let mut cached = seeded_cached_topic(topic, &[], Utc::now());
        cached.dirty = true;
        cached.refresh_scheduled = true;
        let cursor = cached.cursor;
        let payload = serde_json::to_vec(&cached.snapshot_payload).expect("cached payload");

        let reused = reuse_unchanged_cached_topic(&mut cached, &payload);

        assert!(reused.is_some());
        assert!(!cached.dirty);
        assert!(!cached.refresh_scheduled);
        assert_eq!(cached.cursor, cursor);
    }

    #[test]
    fn reused_prompt_cache_baseline_refreshes_reconciliation_bookkeeping() {
        let topic = summary_topic();
        let mut cached = seeded_cached_topic(topic, &[], Utc::now());
        cached.prompt_cache_full_hydration_count = 4;
        cached.prompt_cache_baseline_row_id = 12;
        cached.prompt_cache_response_source = "memory";
        let applied_terminal_ids = HashSet::from(["invoke\0occurred-at".to_string()]);

        finish_prompt_cache_baseline_reuse(
            &mut cached,
            &PromptCacheBaselineBuild {
                baseline_row_id: 37,
                persisted_identities: HashSet::new(),
                runtime_overlay_terminal_identities: HashSet::new(),
            },
            &applied_terminal_ids,
        );

        assert_eq!(cached.prompt_cache_full_hydration_count, 5);
        assert!(cached.prompt_cache_baseline_at.is_some());
        assert_eq!(cached.prompt_cache_baseline_row_id, 37);
        assert_eq!(cached.prompt_cache_response_source, "database_reconcile");
        assert_eq!(
            cached.prompt_cache_applied_terminal_ids,
            applied_terminal_ids
        );
    }

    #[test]
    fn prune_replay_window_drops_expired_entries() {
        let now = Utc::now();
        let topic = summary_topic();
        let descriptor = topic.descriptor();
        let schema_epoch = topic.schema_epoch();
        let mut events = VecDeque::from([
            test_replay_event(
                &descriptor,
                &schema_epoch,
                1,
                now - ChronoDuration::seconds(SUBSCRIPTION_REPLAY_WINDOW_SECS + 5),
                32,
            ),
            test_replay_event(&descriptor, &schema_epoch, 2, now, 32),
        ]);
        let mut total_bytes = 64usize;

        prune_replay_window(&mut events, &mut total_bytes);

        assert_eq!(events.len(), 1);
        assert_eq!(events.front().map(|event| event.frame.cursor), Some(2));
        assert_eq!(total_bytes, 32);
    }

    #[test]
    fn prompt_cache_projection_applies_terminal_delta_without_full_hydration() {
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let mut payload = serde_json::json!({
            "rangeStart": "2026-08-07T00:00:00Z",
            "rangeEnd": "2026-08-08T00:00:00Z",
            "conversations": []
        });
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.id = 7;
        record.invoke_id = "projection-terminal".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("cache-key".to_string());
        record.total_tokens = Some(42);
        record.cost = Some(0.25);
        record.failure_class = Some("none".to_string());

        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build compact delta")
            .expect("prompt cache delta");
        let mut applied_terminal_ids = HashSet::new();
        assert!(
            apply_prompt_cache_records_to_payload(
                &topic,
                &mut payload,
                std::slice::from_ref(&delta),
                &mut applied_terminal_ids,
                0,
            )
            .expect("apply terminal delta")
        );
        let conversation = &payload["conversations"][0];
        assert_eq!(conversation["promptCacheKey"], "cache-key");
        assert_eq!(conversation["requestCount"], 1);
        assert_eq!(conversation["totalTokens"], 42);
        assert_eq!(
            conversation["recentInvocations"].as_array().unwrap().len(),
            1
        );
        assert_eq!(conversation["last24hRequests"].as_array().unwrap().len(), 1);

        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[delta],
            &mut applied_terminal_ids,
            0,
        )
        .expect("deduplicate repeated terminal delta");
        assert_eq!(payload["conversations"][0]["requestCount"], 1);
    }

    #[test]
    fn working_conversations_projection_preserves_stateful_runtime_contract() {
        let now = Utc::now();
        let make_state = |page_size, recent_invocation_limit, blocked_binding_filter| {
            DashboardWorkingConversationsMaterializerState::new(
                PromptCacheConversationsResponse {
                    range_start: format_utc_iso(
                        now - ChronoDuration::minutes(
                            SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                        ),
                    ),
                    range_end: format_utc_iso_precise(now),
                    snapshot_at: Some(format_utc_iso_precise(now)),
                    selection_mode: PromptCacheConversationSelectionMode::ActivityWindow,
                    selected_limit: None,
                    selected_activity_hours: None,
                    selected_activity_minutes: Some(
                        SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                    ),
                    implicit_filter: PromptCacheConversationImplicitFilter {
                        kind: None,
                        filtered_count: 0,
                    },
                    total_matched: Some(0),
                    has_more: false,
                    next_cursor: None,
                    conversations: Vec::new(),
                },
                page_size,
                recent_invocation_limit,
                blocked_binding_filter,
            )
        };
        let local_time = |offset_seconds| {
            format_naive(
                (now - ChronoDuration::seconds(offset_seconds))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            )
        };
        let terminal_delta = |id: i64, invoke_id: &str, prompt_cache_key: &str, offset_seconds| {
            let mut record = dashboard_runtime_topology_live_record(&local_time(offset_seconds));
            record.id = id;
            record.invoke_id = invoke_id.to_string();
            record.prompt_cache_key = Some(prompt_cache_key.to_string());
            record.upstream_account_id = None;
            record.upstream_account_name = None;
            record.status = Some("success".to_string());
            record.live_phase = None;
            record.total_tokens = Some(42);
            record.cost = Some(0.25);
            PromptCacheTopicDelta::from_record(&record)
                .expect("build working terminal delta")
                .expect("working terminal delta")
        };
        let seed_hydrated = |state: &mut DashboardWorkingConversationsMaterializerState,
                             prompt_cache_key: &str,
                             occurred_at: &str,
                             total_matched: i64| {
            assert!(state.replace_hydrated_conversation(
                prompt_cache_key,
                Some(PromptCacheConversationResponse {
                    prompt_cache_key: prompt_cache_key.to_string(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    created_at: occurred_at.to_string(),
                    last_activity_at: occurred_at.to_string(),
                    last_terminal_at: None,
                    last_in_flight_at: None,
                    cursor: None,
                    has_encrypted_session_owner: false,
                    encrypted_owner_account_id: None,
                    encrypted_owner_account_name: None,
                    encrypted_owner_group_name: None,
                    manual_binding: None,
                    blocked_binding: None,
                    upstream_accounts: Vec::new(),
                    recent_invocations: Vec::new(),
                    last24h_requests: Vec::new(),
                }),
            ));
            assert!(state.set_total_matched(total_matched));
        };

        let mut projection = make_state(1, 16, None);
        let baseline_window = (
            projection.response.range_start.clone(),
            projection.response.range_end.clone(),
            projection.response.snapshot_at.clone(),
        );
        let mut applied_terminal_ids = HashSet::new();
        let mut running = dashboard_runtime_topology_live_record(&local_time(30));
        running.id = 1;
        running.invoke_id = "runtime-to-terminal".to_string();
        running.prompt_cache_key = Some("alpha".to_string());
        running.upstream_account_id = None;
        running.upstream_account_name = None;
        let running = PromptCacheTopicDelta::from_record(&running)
            .expect("build running delta")
            .expect("running delta");
        assert_eq!(
            projection
                .apply_deltas(std::slice::from_ref(&running), &mut applied_terminal_ids, 0)
                .expect("missing key requires bounded hydration"),
            WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(BTreeSet::from([
                "alpha".to_string()
            ]))
        );
        seed_hydrated(&mut projection, "alpha", &running.occurred_at, 1);
        assert_eq!(
            projection
                .apply_deltas(&[running], &mut applied_terminal_ids, 0)
                .expect("apply hydrated running delta"),
            WorkingConversationsProjectionUpdate::Changed
        );
        assert_eq!(
            (
                projection.response.range_start.clone(),
                projection.response.range_end.clone(),
                projection.response.snapshot_at.clone(),
            ),
            baseline_window,
            "live projection updates must retain the cursor-consistent baseline window"
        );

        let mut transient = make_state(20, 16, None);
        let mut transient_ids = HashSet::new();
        let mut transient_record = dashboard_runtime_topology_live_record(&local_time(25));
        transient_record.id = 2;
        transient_record.invoke_id = "runtime-removed".to_string();
        transient_record.prompt_cache_key = Some("transient".to_string());
        let transient_upsert = PromptCacheTopicDelta::from_record(&transient_record)
            .expect("build transient runtime delta")
            .expect("transient runtime delta");
        seed_hydrated(
            &mut transient,
            "transient",
            &transient_upsert.occurred_at,
            1,
        );
        assert_eq!(
            transient
                .apply_deltas(&[transient_upsert], &mut transient_ids, 0)
                .expect("apply transient runtime preview"),
            WorkingConversationsProjectionUpdate::Changed
        );
        let RuntimeMutation::Invocation(transient_removal) =
            RuntimeMutation::invocation(&transient_record, RuntimeMutationKind::RuntimeRemoved)
        else {
            unreachable!("runtime removal must produce an invocation mutation");
        };
        let transient_removal =
            PromptCacheTopicDelta::from_runtime_mutation(&transient_removal, None)
                .expect("build transient removal delta")
                .expect("transient removal delta");
        assert_eq!(
            transient
                .apply_deltas(&[transient_removal], &mut transient_ids, 0)
                .expect("remove transient runtime preview"),
            WorkingConversationsProjectionUpdate::Changed
        );
        assert!(transient.response.conversations.is_empty());
        assert_eq!(transient.response.total_matched, Some(0));

        let mut old_in_flight = make_state(20, 16, None);
        let mut old_in_flight_ids = HashSet::new();
        let mut old_in_flight_record = dashboard_runtime_topology_live_record(&local_time(16 * 60));
        old_in_flight_record.id = 3;
        old_in_flight_record.invoke_id = "long-running".to_string();
        old_in_flight_record.prompt_cache_key = Some("long-running".to_string());
        let old_in_flight_delta = PromptCacheTopicDelta::from_record(&old_in_flight_record)
            .expect("build old in-flight delta")
            .expect("old in-flight delta");
        seed_hydrated(
            &mut old_in_flight,
            "long-running",
            &old_in_flight_delta.occurred_at,
            1,
        );
        assert_eq!(
            old_in_flight
                .apply_deltas(&[old_in_flight_delta], &mut old_in_flight_ids, 0)
                .expect("apply old in-flight delta"),
            WorkingConversationsProjectionUpdate::Changed
        );
        assert_eq!(old_in_flight.response.conversations.len(), 1);
        assert!(
            !old_in_flight.expire(now),
            "in-flight conversations remain in the working set regardless of age"
        );
        assert_eq!(old_in_flight.response.conversations.len(), 1);

        let terminal = terminal_delta(1, "runtime-to-terminal", "alpha", 30);
        assert_eq!(
            projection
                .apply_deltas(
                    std::slice::from_ref(&terminal),
                    &mut applied_terminal_ids,
                    0
                )
                .expect("replace runtime record with terminal"),
            WorkingConversationsProjectionUpdate::Changed
        );
        let alpha = projection
            .response
            .conversations
            .iter()
            .find(|conversation| conversation.prompt_cache_key == "alpha")
            .expect("alpha conversation");
        assert_eq!(alpha.request_count, 1);
        assert_eq!(alpha.total_tokens, 42);
        assert!(alpha.last_in_flight_at.is_none());
        assert_eq!(alpha.last24h_requests.len(), 1);
        assert_eq!(alpha.upstream_accounts[0].upstream_account_id, None);
        let same_second_terminal = terminal_delta(4, "same-second-terminal", "alpha", 30);
        assert_eq!(
            projection
                .apply_deltas(&[same_second_terminal], &mut applied_terminal_ids, 0)
                .expect("apply distinct terminal in the same persisted second"),
            WorkingConversationsProjectionUpdate::Changed
        );
        let alpha = projection
            .response
            .conversations
            .iter()
            .find(|conversation| conversation.prompt_cache_key == "alpha")
            .expect("alpha conversation with same-second terminals");
        assert_eq!(alpha.request_count, 2);
        assert_eq!(alpha.total_tokens, 84);
        assert_eq!(alpha.last24h_requests.len(), 2);
        assert_eq!(
            alpha
                .last24h_requests
                .iter()
                .map(|point| point.cumulative_tokens)
                .collect::<Vec<_>>(),
            vec![42, 84],
            "distinct invocations in the same persisted second must retain both chart points",
        );
        assert_eq!(
            projection
                .apply_deltas(&[terminal], &mut applied_terminal_ids, 0)
                .expect("deduplicate terminal replay"),
            WorkingConversationsProjectionUpdate::Unchanged
        );
        assert_eq!(projection.response.conversations[0].request_count, 2);

        let mut alpha_runtime_record = dashboard_runtime_topology_live_record(&local_time(10));
        alpha_runtime_record.id = 3;
        alpha_runtime_record.invoke_id = "alpha-runtime-preview".to_string();
        alpha_runtime_record.prompt_cache_key = Some("alpha".to_string());
        let alpha_runtime_preview = PromptCacheTopicDelta::from_record(&alpha_runtime_record)
            .expect("build alpha runtime preview")
            .expect("alpha runtime preview");
        assert_eq!(
            projection
                .apply_deltas(&[alpha_runtime_preview], &mut applied_terminal_ids, 0)
                .expect("apply alpha runtime preview"),
            WorkingConversationsProjectionUpdate::Changed
        );
        let RuntimeMutation::Invocation(alpha_runtime_removal) =
            RuntimeMutation::invocation(&alpha_runtime_record, RuntimeMutationKind::RuntimeRemoved)
        else {
            unreachable!("runtime removal must produce an invocation mutation");
        };
        let alpha_runtime_removal =
            PromptCacheTopicDelta::from_runtime_mutation(&alpha_runtime_removal, None)
                .expect("build alpha runtime removal")
                .expect("alpha runtime removal");
        let terminal_activity_at = projection.response.conversations[0]
            .last_terminal_at
            .clone()
            .expect("alpha terminal activity");
        assert_eq!(
            projection
                .apply_deltas(&[alpha_runtime_removal], &mut applied_terminal_ids, 0)
                .expect("remove alpha runtime preview"),
            WorkingConversationsProjectionUpdate::Changed
        );
        let alpha = &projection.response.conversations[0];
        assert_eq!(alpha.last_in_flight_at, None);
        assert_eq!(alpha.last_activity_at, terminal_activity_at);

        let mut account_history = make_state(20, 16, None);
        let mut account_history_ids = HashSet::new();
        let account_history_delta =
            terminal_delta(200, "account-history-base", "account-history", 3);
        seed_hydrated(
            &mut account_history,
            "account-history",
            &account_history_delta.occurred_at,
            1,
        );
        account_history.response.conversations[0].upstream_accounts = (1..=3)
            .map(
                |upstream_account_id| PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: Some(upstream_account_id),
                    upstream_account_name: Some(format!("Historical {upstream_account_id}")),
                    request_count: 10,
                    total_tokens: 100,
                    total_cost: 1.0,
                    last_activity_at: account_history_delta.occurred_at.clone(),
                },
            )
            .collect();
        let mut omitted_account_delta =
            terminal_delta(201, "omitted-account-live", "account-history", 2);
        omitted_account_delta.upstream_account_id = Some(4);
        omitted_account_delta.upstream_account_name = Some("Historical 4".to_string());
        assert_eq!(
            account_history
                .apply_deltas(&[omitted_account_delta], &mut account_history_ids, 0,)
                .expect("omitted historical account requires bounded hydration"),
            WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(BTreeSet::from([
                "account-history".to_string()
            ]))
        );
        assert_eq!(
            account_history.response.conversations[0]
                .upstream_accounts
                .len(),
            PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT,
            "a partial live delta must not replace a capped historical account summary",
        );

        let beta = terminal_delta(2, "newer-terminal", "beta", 5);
        seed_hydrated(&mut projection, "beta", &beta.occurred_at, 2);
        assert_eq!(
            projection
                .apply_deltas(&[beta], &mut applied_terminal_ids, 0)
                .expect("apply newer page candidate"),
            WorkingConversationsProjectionUpdate::Changed
        );
        assert_eq!(projection.response.total_matched, Some(2));
        assert!(projection.response.has_more);
        assert_eq!(projection.response.conversations.len(), 1);
        assert_eq!(
            projection.response.conversations[0].prompt_cache_key,
            "beta"
        );
        assert!(
            projection.response.next_cursor.is_none(),
            "a live page replacement must not manufacture a cursor for the cold baseline"
        );
        assert!(
            projection.response.conversations[0].cursor.is_none(),
            "a live-only page member has no valid cursor in the cold baseline"
        );

        let binding = PromptCacheConversationBindingResponse {
            prompt_cache_key: "beta".to_string(),
            binding_kind: "upstream_account".to_string(),
            group_name: None,
            upstream_account_id: Some(9),
            upstream_account_name: Some("Pinned account".to_string()),
            has_encrypted_session_owner: true,
            encrypted_owner_account_id: Some(11),
            encrypted_owner_account_name: Some("Owner account".to_string()),
            encrypted_owner_group_name: Some("Owner group".to_string()),
            sticky_routes: Vec::new(),
            timeouts: RoutingTimeoutSettings::default(),
            timeout_field_sources: RoutingTimeoutFieldSources {
                responses_first_byte_timeout_secs: "root".to_string(),
                compact_first_byte_timeout_secs: "root".to_string(),
                image_first_byte_timeout_secs: "root".to_string(),
                responses_stream_timeout_secs: "root".to_string(),
                compact_stream_timeout_secs: "root".to_string(),
            },
            allow_switch_upstream: None,
            fast_mode_rewrite_mode: None,
            image_tool_rewrite_mode: None,
            codex_imagegen_rewrite_mode: None,
            available_models: None,
            available_models_mode: None,
            forward_proxy_key: None,
            forward_proxy_keys: Vec::new(),
            policy_field_sources: PromptCacheConversationPolicyFieldSources {
                allow_switch_upstream: "root".to_string(),
                fast_mode_rewrite_mode: "root".to_string(),
                image_tool_rewrite_mode: "root".to_string(),
                codex_imagegen_rewrite_mode: "root".to_string(),
                available_models: "root".to_string(),
                available_models_mode: "root".to_string(),
                forward_proxy_key: "root".to_string(),
            },
            updated_at: None,
        };
        assert_eq!(projection.apply_binding("beta", &binding), Some(true));
        let beta = &projection.response.conversations[0];
        assert_eq!(beta.encrypted_owner_account_id, Some(11));
        assert_eq!(
            beta.manual_binding
                .as_ref()
                .map(|value| value.upstream_account_id),
            Some(Some(9))
        );

        let blocked_filter = PromptCacheConversationBlockedBindingFilter {
            upstream_account_id: Some(7),
            constraint_source: Some(BlockedBindingConstraintSource::UpstreamAccountBinding),
        };
        let mut filtered = make_state(20, 16, Some(blocked_filter));
        let mut filtered_ids = HashSet::new();
        let mut mismatched = terminal_delta(3, "blocked-mismatch", "blocked", 4);
        mismatched
            .preview
            .as_mut()
            .expect("mismatched preview")
            .blocked_binding = Some(BlockedBindingDiagnostic {
            constraint_source: BlockedBindingConstraintSource::UpstreamAccountBinding,
            upstream_account_id: 8,
            upstream_account_label: "Other account".to_string(),
            prompt_cache_key: Some("blocked".to_string()),
            recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
        });
        assert_eq!(
            filtered
                .apply_deltas(&[mismatched], &mut filtered_ids, 0)
                .expect("reject mismatched blocked binding"),
            WorkingConversationsProjectionUpdate::Unchanged
        );
        let mut matched = terminal_delta(4, "blocked-match", "blocked", 3);
        matched
            .preview
            .as_mut()
            .expect("matched preview")
            .blocked_binding = Some(BlockedBindingDiagnostic {
            constraint_source: BlockedBindingConstraintSource::UpstreamAccountBinding,
            upstream_account_id: 7,
            upstream_account_label: "Matched account".to_string(),
            prompt_cache_key: Some("blocked".to_string()),
            recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
        });
        assert_eq!(
            filtered
                .apply_deltas(std::slice::from_ref(&matched), &mut filtered_ids, 0)
                .expect("matching blocked binding requires bounded hydration"),
            WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(BTreeSet::from([
                "blocked".to_string()
            ]))
        );
        seed_hydrated(&mut filtered, "blocked", &matched.occurred_at, 1);
        assert_eq!(
            filtered
                .apply_deltas(&[matched], &mut filtered_ids, 0)
                .expect("accept hydrated matching blocked binding"),
            WorkingConversationsProjectionUpdate::Changed
        );
        assert_eq!(filtered.response.conversations.len(), 1);

        let mut recent = make_state(20, 16, None);
        let mut recent_ids = HashSet::new();
        for index in 0..17 {
            let delta = terminal_delta(
                100 + index,
                &format!("recent-{index:02}"),
                "recent",
                17 - index,
            );
            if index == 0 {
                seed_hydrated(&mut recent, "recent", &delta.occurred_at, 1);
            }
            recent
                .apply_deltas(&[delta], &mut recent_ids, 0)
                .expect("apply ordered recent terminal");
        }
        let recent_conversation = &recent.response.conversations[0];
        assert_eq!(recent_conversation.recent_invocations.len(), 16);
        assert_eq!(
            recent_conversation.recent_invocations[0].invoke_id,
            "recent-16"
        );
        assert_eq!(recent_conversation.request_count, 17);
        let mut expired = recent_conversation.clone();
        expired.prompt_cache_key = "expired".to_string();
        expired.last_activity_at = format_utc_iso(
            now - ChronoDuration::minutes(
                SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES + 1,
            ),
        );
        recent.response.conversations.push(expired);
        *recent
            .response
            .total_matched
            .as_mut()
            .expect("tracked total") += 1;
        recent.response.conversations[0].last24h_requests.push(
            PromptCacheConversationRequestPointResponse {
                occurred_at: format_utc_iso(now - ChronoDuration::hours(25)),
                status: "success".to_string(),
                is_success: true,
                outcome: "success".to_string(),
                request_tokens: 1,
                cumulative_tokens: 43,
            },
        );
        assert!(recent.expire(now));
        assert!(
            recent
                .response
                .conversations
                .iter()
                .all(|conversation| conversation.prompt_cache_key != "expired")
        );
        assert_eq!(recent.response.total_matched, Some(1));
        assert!(
            recent.response.conversations[0]
                .last24h_requests
                .iter()
                .all(|point| parse_to_utc_datetime(&point.occurred_at)
                    .is_some_and(|occurred_at| occurred_at >= now - ChronoDuration::hours(24)))
        );
    }

    #[test]
    fn typed_runtime_removal_clears_the_matching_prompt_cache_preview() {
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let mut payload = serde_json::json!({ "conversations": [] });
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.invoke_id = "runtime-removal".to_string();
        record.prompt_cache_key = Some("cache-key".to_string());
        record.status = Some("running".to_string());

        let upsert = PromptCacheTopicDelta::from_record(&record)
            .expect("build compact runtime delta")
            .expect("prompt cache runtime delta");
        let RuntimeMutation::Invocation(removal) =
            RuntimeMutation::invocation(&record, RuntimeMutationKind::RuntimeRemoved)
        else {
            unreachable!("runtime removal must produce an invocation mutation");
        };
        let removal = PromptCacheTopicDelta::from_runtime_mutation(&removal, None)
            .expect("build compact removal delta")
            .expect("prompt cache removal delta");
        let mut applied_terminal_ids = HashSet::new();

        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[upsert],
            &mut applied_terminal_ids,
            0,
        )
        .expect("apply runtime preview");
        assert_eq!(
            payload["conversations"][0]["recentInvocations"]
                .as_array()
                .expect("recent previews")
                .len(),
            1
        );

        assert!(
            apply_prompt_cache_records_to_payload(
                &topic,
                &mut payload,
                &[removal],
                &mut applied_terminal_ids,
                0,
            )
            .expect("remove runtime preview")
        );
        assert!(
            payload["conversations"][0]["recentInvocations"]
                .as_array()
                .expect("recent previews")
                .is_empty()
        );
    }

    #[test]
    fn prompt_cache_sticky_projection_uses_sticky_key_without_prompt_outcome() {
        let topic = SubscriptionTopic::PromptCacheStickyWindow {
            account_id: 9,
            selection: AccountStickyKeySelection::Count(20),
        };
        let mut payload = serde_json::json!({ "conversations": [] });
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.invoke_id = "sticky-terminal".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("prompt-key".to_string());
        record.sticky_key = Some("sticky-key".to_string());
        record.upstream_account_id = Some(9);
        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build compact delta")
            .expect("sticky delta");
        let mut applied_terminal_ids = HashSet::new();

        assert!(
            apply_prompt_cache_records_to_payload(
                &topic,
                &mut payload,
                &[delta],
                &mut applied_terminal_ids,
                0,
            )
            .expect("apply sticky delta")
        );
        let conversation = &payload["conversations"][0];
        assert_eq!(conversation["stickyKey"], "sticky-key");
        assert!(conversation["last24hRequests"][0].get("outcome").is_none());
    }

    #[test]
    fn prompt_cache_terminal_dedup_survives_recent_truncation() {
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(1),
        };
        let mut payload = serde_json::json!({ "conversations": [] });
        let mut first = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        first.invoke_id = "first-terminal".to_string();
        first.status = Some("success".to_string());
        first.live_phase = None;
        first.prompt_cache_key = Some("cache-key".to_string());
        let first = PromptCacheTopicDelta::from_record(&first)
            .expect("build first delta")
            .expect("first delta");
        let mut second = dashboard_runtime_topology_live_record("2026-08-08 10:01:00");
        second.invoke_id = "second-terminal".to_string();
        second.status = Some("success".to_string());
        second.live_phase = None;
        second.prompt_cache_key = Some("cache-key".to_string());
        let second = PromptCacheTopicDelta::from_record(&second)
            .expect("build second delta")
            .expect("second delta");
        let mut applied_terminal_ids = HashSet::new();

        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[first.clone(), second],
            &mut applied_terminal_ids,
            0,
        )
        .expect("apply terminal deltas");
        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[first],
            &mut applied_terminal_ids,
            0,
        )
        .expect("replay truncated terminal");

        assert_eq!(payload["conversations"][0]["requestCount"], 2);
        assert_eq!(
            payload["conversations"][0]["recentInvocations"]
                .as_array()
                .expect("recent invocations")
                .len(),
            1
        );
    }

    #[test]
    fn prompt_cache_baseline_cursor_skips_recovered_terminal_totals() {
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let mut payload = serde_json::json!({ "conversations": [] });
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.id = 7;
        record.invoke_id = "recovered-terminal".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("cache-key".to_string());
        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build recovered delta")
            .expect("recovered delta");
        let mut applied_terminal_ids = HashSet::new();

        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[delta],
            &mut applied_terminal_ids,
            7,
        )
        .expect("apply recovered delta");

        assert_eq!(payload["conversations"][0]["requestCount"], 0);
        assert!(applied_terminal_ids.is_empty());
        assert_eq!(
            payload["conversations"][0]["recentInvocations"]
                .as_array()
                .expect("recent invocations")
                .len(),
            1
        );
    }

    #[test]
    fn prompt_cache_baseline_does_not_replay_an_identity_already_in_payload() {
        let mut record = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        record.id = 0;
        record.invoke_id = "persisted-during-baseline".to_string();
        record.status = Some("success".to_string());
        record.live_phase = None;
        record.prompt_cache_key = Some("cache-key".to_string());
        let delta = PromptCacheTopicDelta::from_record(&record)
            .expect("build concurrent delta")
            .expect("prompt cache delta");

        assert!(prompt_cache_delta_needs_replay(&delta, &HashSet::new()));
        assert!(!prompt_cache_delta_needs_replay(
            &delta,
            &HashSet::from([delta.identity.clone()]),
        ));
    }

    #[test]
    fn prompt_cache_projection_keeps_latest_activity_for_out_of_order_records() {
        let topic = SubscriptionTopic::PromptCacheWindow {
            selection: PromptCacheConversationSelection::Count(20),
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: Some(16),
        };
        let mut payload = serde_json::json!({ "conversations": [] });
        let mut newer = dashboard_runtime_topology_live_record("2026-08-08 10:01:00");
        newer.invoke_id = "newer".to_string();
        newer.prompt_cache_key = Some("cache-key".to_string());
        newer.status = Some("success".to_string());
        newer.live_phase = None;
        newer.upstream_account_id = Some(9);
        let newer = PromptCacheTopicDelta::from_record(&newer)
            .expect("build newer delta")
            .expect("newer delta");
        let mut older = dashboard_runtime_topology_live_record("2026-08-08 10:00:00");
        older.invoke_id = "older".to_string();
        older.prompt_cache_key = Some("cache-key".to_string());
        older.status = Some("success".to_string());
        older.live_phase = None;
        older.upstream_account_id = Some(9);
        let older = PromptCacheTopicDelta::from_record(&older)
            .expect("build older delta")
            .expect("older delta");
        let mut applied_terminal_ids = HashSet::new();

        apply_prompt_cache_records_to_payload(
            &topic,
            &mut payload,
            &[newer, older],
            &mut applied_terminal_ids,
            0,
        )
        .expect("apply out-of-order deltas");

        assert_eq!(
            payload["conversations"][0]["lastActivityAt"],
            "2026-08-08T02:01:00Z"
        );
        assert_eq!(
            payload["conversations"][0]["createdAt"],
            "2026-08-08T02:00:00Z"
        );
        assert_eq!(
            payload["conversations"][0]["lastTerminalAt"],
            "2026-08-08T02:01:00Z"
        );
        assert_eq!(
            payload["conversations"][0]["upstreamAccounts"][0]["lastActivityAt"],
            "2026-08-08T02:01:00Z"
        );
    }

    #[test]
    fn prompt_cache_binding_patch_updates_only_the_selected_conversation() {
        let mut payload = serde_json::json!({
            "conversations": [
                { "promptCacheKey": "selected", "hasEncryptedSessionOwner": false },
                { "promptCacheKey": "other", "hasEncryptedSessionOwner": false }
            ]
        });
        let binding = serde_json::json!({
            "bindingKind": "account",
            "groupName": "primary",
            "upstreamAccountId": 9,
            "upstreamAccountName": "Account 9",
            "hasEncryptedSessionOwner": true,
            "encryptedOwnerAccountId": 8,
            "encryptedOwnerAccountName": "Account 8",
            "encryptedOwnerGroupName": "owners"
        });

        assert_eq!(
            patch_prompt_cache_binding_payload(&mut payload, "selected", &binding),
            Some(true)
        );
        assert_eq!(
            payload["conversations"][0]["manualBinding"]["upstreamAccountId"],
            9
        );
        assert_eq!(
            payload["conversations"][0]["hasEncryptedSessionOwner"],
            true
        );
        assert_eq!(
            payload["conversations"][1]["hasEncryptedSessionOwner"],
            false
        );
    }

    #[test]
    fn subscription_event_envelope_serializes_camel_case_fields() {
        let payload = SubscriptionEventEnvelope::Snapshot {
            topic: SubscriptionTopicDescriptor {
                topic: "app.version".to_string(),
                params: BTreeMap::new(),
            },
            topic_key: "topic-key".to_string(),
            schema_epoch: "app.version/v1".to_string(),
            cursor: 7,
            payload: json!({
                "backend": "0.2.0-dev",
                "frontend": "0.2.0-dev",
            }),
        };

        let encoded = serde_json::to_value(payload).expect("serialize envelope");

        assert_eq!(encoded.get("topicKey"), Some(&json!("topic-key")));
        assert_eq!(encoded.get("schemaEpoch"), Some(&json!("app.version/v1")));
        assert!(encoded.get("topic_key").is_none());
        assert!(encoded.get("schema_epoch").is_none());
    }

    #[test]
    fn serialized_topic_frame_preserves_snapshot_replay_and_live_wire_kinds() {
        let topic = summary_topic();
        let frame = serialize_topic_frame(
            topic.descriptor(),
            topic.cache_key().expect("topic key"),
            topic.schema_epoch(),
            7,
            serde_json::to_vec(&json!({ "total": 3 })).expect("payload"),
        )
        .expect("serialized frame");

        for (kind, expected) in [
            (TopicFrameKind::Snapshot, "snapshot"),
            (TopicFrameKind::Replay, "replay"),
            (TopicFrameKind::Live, "live"),
        ] {
            let chunks = frame.event_chunks(kind);
            let wire = chunks.concat();
            let envelope: Value =
                serde_json::from_slice(&wire[6..wire.len() - 2]).expect("wire envelope");
            assert_eq!(envelope["type"], expected);
            assert_eq!(envelope["cursor"], 7);
            assert_eq!(envelope["payload"], json!({ "total": 3 }));
        }
    }

    #[test]
    fn decode_resume_query_accepts_legacy_topic_key_format() {
        let descriptor = summary_topic().descriptor();
        let topic_key = summary_topic().cache_key().expect("topic key");
        let raw = serde_json::to_string(&vec![json!({
            "topicKey": topic_key.clone(),
            "cursor": 4,
            "schemaEpoch": "stats.summary.current/v1",
        })])
        .expect("encode resume query");

        let decoded = decode_resume_query(Some(&raw), &[descriptor]).expect("decode resume");

        assert_eq!(
            decoded,
            vec![SubscriptionResumeCursor {
                topic_key,
                cursor: 4,
                schema_epoch: "stats.summary.current/v1".to_string(),
            }]
        );
    }

    #[test]
    fn decode_resume_query_accepts_compact_topic_index_format() {
        let descriptor = summary_topic().descriptor();
        let topic_key = summary_topic().cache_key().expect("topic key");
        let raw = serde_json::to_string(&vec![json!({
            "topicIndex": 0,
            "cursor": 4,
            "schemaEpoch": "stats.summary.current/v1",
        })])
        .expect("encode resume query");

        let decoded = decode_resume_query(Some(&raw), &[descriptor]).expect("decode resume");

        assert_eq!(
            decoded,
            vec![SubscriptionResumeCursor {
                topic_key,
                cursor: 4,
                schema_epoch: "stats.summary.current/v1".to_string(),
            }]
        );
    }

    #[test]
    fn decode_resume_query_rejects_out_of_range_compact_topic_index() {
        let descriptor = summary_topic().descriptor();
        let raw = serde_json::to_string(&vec![json!({
            "topicIndex": 1,
            "cursor": 4,
            "schemaEpoch": "stats.summary.current/v1",
        })])
        .expect("encode resume query");

        let error =
            decode_resume_query(Some(&raw), &[descriptor]).expect_err("resume should reject");

        match error {
            ApiError::BadRequest(err) => {
                assert!(format!("{err}").contains("resume topicIndex out of range: 1"));
            }
            other => panic!("expected bad request, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn replay_returns_gap_when_cursor_is_within_window() {
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let topic_key = topic.cache_key().expect("topic key");
        let schema_epoch = topic.schema_epoch();
        let cached = seeded_cached_topic(topic, &[1, 2, 3, 4], Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let replay = hub
            .replay_events_for_resume(
                &topic_key,
                schema_epoch.clone(),
                Some(&SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 2,
                    schema_epoch,
                }),
            )
            .await
            .expect("replay should be eligible")
            .expect("replay gap should exist");

        assert_eq!(
            replay
                .iter()
                .map(|event| event.frame.cursor)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[tokio::test]
    async fn replay_rejects_schema_epoch_mismatch() {
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let topic_key = topic.cache_key().expect("topic key");
        let cached = seeded_cached_topic(topic, &[1, 2], Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let result = hub
            .replay_events_for_resume(
                &topic_key,
                "stats.summary.current/v1".to_string(),
                Some(&SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 1,
                    schema_epoch: "stats.summary.current/v0".to_string(),
                }),
            )
            .await;

        assert!(matches!(result, Err(ReplayMissReason::SchemaEpochMismatch)));
    }

    #[tokio::test]
    async fn replay_rejects_window_miss_when_cursor_is_older_than_front() {
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let topic_key = topic.cache_key().expect("topic key");
        let schema_epoch = topic.schema_epoch();
        let cached = seeded_cached_topic(topic, &[10, 11, 12], Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let result = hub
            .replay_events_for_resume(
                &topic_key,
                schema_epoch.clone(),
                Some(&SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 8,
                    schema_epoch,
                }),
            )
            .await;

        assert!(matches!(result, Err(ReplayMissReason::GapWindowMiss)));
    }

    #[tokio::test]
    async fn replay_rejects_gaps_that_exceed_event_budget() {
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let topic_key = topic.cache_key().expect("topic key");
        let schema_epoch = topic.schema_epoch();
        let cursors = (1..=(SUBSCRIPTION_REPLAY_MAX_GAP_EVENTS as u64 + 2)).collect::<Vec<_>>();
        let cached = seeded_cached_topic(topic, &cursors, Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let result = hub
            .replay_events_for_resume(
                &topic_key,
                schema_epoch.clone(),
                Some(&SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 1,
                    schema_epoch,
                }),
            )
            .await;

        assert!(matches!(
            result,
            Err(ReplayMissReason::GapEventBudgetExceeded)
        ));
    }

    #[tokio::test]
    async fn prepare_connection_reports_snapshot_without_resume() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let descriptor = topic.descriptor();
        let topic_key = topic.cache_key().expect("topic key");
        let cached = seeded_cached_topic(topic, &[1, 2, 3], Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let prepared = hub
            .prepare_connection(state, vec![descriptor], Vec::new())
            .await
            .expect("prepare connection");

        assert_eq!(prepared.initial.len(), 1);
        assert_eq!(
            prepared.outcomes,
            vec![TopicInitOutcome {
                topic_key,
                disposition: TopicInitDisposition::SnapshotNoResume,
                replay_event_count: 0,
                replay_bytes: 0,
                cursor: 3,
                miss_reason: None,
            }]
        );
    }

    #[tokio::test]
    async fn prepare_connection_reports_replay_hit_and_resume_caught_up() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let descriptor = topic.descriptor();
        let topic_key = topic.cache_key().expect("topic key");
        let schema_epoch = topic.schema_epoch();
        let cached = seeded_cached_topic(topic, &[1, 2, 3, 4], Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let replay_hit = hub
            .prepare_connection(
                state.clone(),
                vec![descriptor.clone()],
                vec![SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 2,
                    schema_epoch: schema_epoch.clone(),
                }],
            )
            .await
            .expect("prepare connection");
        assert_eq!(replay_hit.initial.len(), 2);
        assert_eq!(
            replay_hit.outcomes[0],
            TopicInitOutcome {
                topic_key: topic_key.clone(),
                disposition: TopicInitDisposition::ReplayHit,
                replay_event_count: 2,
                replay_bytes: 64,
                cursor: 4,
                miss_reason: None,
            }
        );

        let caught_up = hub
            .prepare_connection(
                state,
                vec![descriptor],
                vec![SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 4,
                    schema_epoch,
                }],
            )
            .await
            .expect("prepare connection");
        assert!(caught_up.initial.is_empty());
        assert_eq!(
            caught_up.outcomes[0],
            TopicInitOutcome {
                topic_key,
                disposition: TopicInitDisposition::ResumeCaughtUp,
                replay_event_count: 0,
                replay_bytes: 0,
                cursor: 4,
                miss_reason: None,
            }
        );
    }

    #[tokio::test]
    async fn prepare_connection_reports_snapshot_resume_miss() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let hub = SubscriptionHub::new();
        let topic = summary_topic();
        let descriptor = topic.descriptor();
        let topic_key = topic.cache_key().expect("topic key");
        let cached = seeded_cached_topic(topic, &[1, 2, 3], Utc::now());
        hub.state
            .lock()
            .await
            .topics
            .insert(topic_key.clone(), cached);

        let prepared = hub
            .prepare_connection(
                state,
                vec![descriptor],
                vec![SubscriptionResumeCursor {
                    topic_key: topic_key.clone(),
                    cursor: 2,
                    schema_epoch: "stats.summary.current/v0".to_string(),
                }],
            )
            .await
            .expect("prepare connection");

        assert_eq!(prepared.initial.len(), 1);
        assert_eq!(
            prepared.outcomes,
            vec![TopicInitOutcome {
                topic_key,
                disposition: TopicInitDisposition::SnapshotResumeMiss,
                replay_event_count: 0,
                replay_bytes: 0,
                cursor: 3,
                miss_reason: Some("schema_epoch_mismatch"),
            }]
        );
    }

    #[tokio::test]
    async fn invalidate_dashboard_activity_snapshot_cache_only_removes_selected_entry() {
        let cache = Arc::new(Mutex::new(DashboardActivitySnapshotCacheState::default()));
        let selection_a = DashboardActivitySnapshotSelection {
            range: "today".to_string(),
            range_anchor: "2026-07-20".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            source_scope: "all".to_string(),
            recent_limit: 4,
            include_accounts: true,
            include_recent: true,
        };
        let selection_b = DashboardActivitySnapshotSelection {
            range: "7d".to_string(),
            range_anchor: "rolling".to_string(),
            time_zone: SUBSCRIPTION_DEFAULT_TIME_ZONE.to_string(),
            source_scope: "all".to_string(),
            recent_limit: 8,
            include_accounts: true,
            include_recent: true,
        };
        let (signal_a, mut rx_a) = watch::channel(false);
        let (signal_b, rx_b) = watch::channel(false);

        {
            let mut guard = cache.lock().await;
            guard.entries.insert(
                selection_a.clone(),
                DashboardActivitySnapshotCacheEntry {
                    cached_at: Instant::now(),
                    last_reconcile_attempted_at: Instant::now(),
                    last_reconcile_failed: false,
                    baseline_snapshot_cursor: 0,
                    expiry_covered_until: None,
                    expiry_terminal_deltas: VecDeque::new(),
                    expiry_delta_estimated_bytes: 0,
                    response: DashboardActivitySnapshot::test_stub("today"),
                },
            );
            guard.entries.insert(
                selection_b.clone(),
                DashboardActivitySnapshotCacheEntry {
                    cached_at: Instant::now(),
                    last_reconcile_attempted_at: Instant::now(),
                    last_reconcile_failed: false,
                    baseline_snapshot_cursor: 0,
                    expiry_covered_until: None,
                    expiry_terminal_deltas: VecDeque::new(),
                    expiry_delta_estimated_bytes: 0,
                    response: DashboardActivitySnapshot::test_stub("7d"),
                },
            );
            guard.in_flight.insert(
                selection_a.clone(),
                DashboardActivitySnapshotInFlight {
                    signal: signal_a,
                    waiter_count: 2,
                    baseline_cursor: None,
                },
            );
            guard.in_flight.insert(
                selection_b.clone(),
                DashboardActivitySnapshotInFlight {
                    signal: signal_b,
                    waiter_count: 3,
                    baseline_cursor: None,
                },
            );
        }

        invalidate_dashboard_activity_snapshot_cache(
            cache.as_ref(),
            &selection_a,
            "scheduled_terminal_refresh",
        )
        .await;

        rx_a.changed()
            .await
            .expect("selected in-flight should be signaled");
        assert!(rx_a.borrow().to_owned());
        assert!(!*rx_b.borrow());

        let guard = cache.lock().await;
        assert!(!guard.entries.contains_key(&selection_a));
        assert!(guard.entries.contains_key(&selection_b));
        assert!(!guard.in_flight.contains_key(&selection_a));
        assert!(guard.in_flight.contains_key(&selection_b));
    }

    #[tokio::test]
    async fn dashboard_activity_live_overlay_clears_stale_optional_latency_fields() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let now = Utc::now();
        let mut payload = json!({
            "rangeStart": now.to_rfc3339(),
            "rangeEnd": (now + ChronoDuration::minutes(5)).to_rfc3339(),
            "summary": {
                "stats": {
                    "inProgressConversationCount": 3,
                    "inProgressRetryConversationCount": 2,
                    "inProgressPhaseCounts": {}
                },
                "modelPerformance": {
                    "available": false
                },
                "tokensPerMinute": 123.0,
                "spendRate": 4.56,
                "currentFirstResponseByteTotalAvgMs": 789.0,
                "currentFirstTokenAvgMs": 987.0,
                "currentAvgTotalMs": 456.0,
                "currentAvgResponseMs": 135.0
            },
            "accounts": [{
                "accountKey": "upstream:42",
                "upstreamAccountId": 42,
                "requestCount": 9,
                "tokensPerMinute": 33.0,
                "spendRate": 1.23,
                "currentFirstResponseByteTotalAvgMs": 654.0,
                "currentFirstTokenAvgMs": 456.0,
                "currentAvgTotalMs": 321.0,
                "currentAvgResponseMs": 246.0,
                "recentInvocations": []
            }]
        });
        let live = DashboardActivityLiveSnapshot {
            revision: 7,
            generated_at: now.to_rfc3339(),
            in_progress_invocation_count: 0,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            network_live_bucket: None,
            network_realtime_rate: None,
            accounts: Vec::new(),
        };

        let applied =
            apply_dashboard_activity_live_overlay_to_payload(state.as_ref(), &mut payload, &live)
                .expect("apply dashboard activity live overlay");

        assert!(applied);
        let summary = payload
            .get("summary")
            .and_then(Value::as_object)
            .expect("summary object");
        assert_eq!(
            summary.get("currentFirstResponseByteTotalAvgMs"),
            None,
            "summary stale currentFirstResponseByteTotalAvgMs should be removed",
        );
        assert_eq!(
            summary.get("currentFirstTokenAvgMs"),
            None,
            "summary stale currentFirstTokenAvgMs should be removed",
        );
        assert_eq!(
            summary.get("currentAvgTotalMs"),
            None,
            "summary stale currentAvgTotalMs should be removed",
        );
        assert_eq!(
            summary.get("currentAvgResponseMs"),
            None,
            "summary stale currentAvgResponseMs should be removed",
        );

        let account = payload
            .get("accounts")
            .and_then(Value::as_array)
            .and_then(|accounts| accounts.first())
            .and_then(Value::as_object)
            .expect("account object");
        assert_eq!(
            account.get("currentFirstResponseByteTotalAvgMs"),
            None,
            "account stale currentFirstResponseByteTotalAvgMs should be removed",
        );
        assert_eq!(
            account.get("currentFirstTokenAvgMs"),
            None,
            "account stale currentFirstTokenAvgMs should be removed",
        );
        assert_eq!(
            account.get("currentAvgTotalMs"),
            None,
            "account stale currentAvgTotalMs should be removed",
        );
        assert_eq!(
            account.get("currentAvgResponseMs"),
            None,
            "account stale currentAvgResponseMs should be removed",
        );
    }
}
