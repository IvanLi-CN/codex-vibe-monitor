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
    prompt_cache_refresh_scheduled: bool,
    prompt_cache_reconcile_scheduled: bool,
    prompt_cache_pending_records: BTreeMap<String, PromptCacheTopicDelta>,
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
    PromptCacheProjection,
    PromptCacheWindow,
    PromptCacheStickyWindow,
    Attempt(String),
    Binding(String),
    HistoryPromptCacheKey(String),
    HistoryStickyKey(String),
    StickyRoute(String),
}

#[derive(Debug, Clone)]
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

struct PromptCacheBaselineBuild {
    baseline_row_id: i64,
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
            Self::NetworkTimeseries { .. } | Self::NetworkRecent { .. } => false,
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
            snapshot.live_path_db_read_count = snapshot
                .live_path_db_read_count
                .saturating_add(cached.prompt_cache_full_hydration_count.saturating_sub(1))
                .saturating_add(cached.prompt_cache_bounded_key_hydration_count);
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
            if resume_cursor.is_some() {
                self.dashboard_topology_counters
                    .record_reconnect_churn(topic.name());
            }
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
            let persisted_identities = load_persisted_prompt_cache_identities(
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
                    },
                ));
            }
        }
        Err(ApiError::from(anyhow!(
            "prompt cache baseline changed during build"
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
        let (mut built_payload, prompt_cache_build) = if is_prompt_cache_topic {
            let (payload, build) = self
                .build_prompt_cache_consistent_baseline(state.clone(), &topic)
                .await?;
            (payload, Some(build))
        } else {
            (topic.build_cached_payload(state.clone()).await?, None)
        };
        self.dashboard_topology_counters.record_materialization(
            topic.name(),
            matches!(&built_payload, BuiltSubscriptionTopicPayload::Json(_))
                && topic.is_unmigrated_dashboard_hot_projection(),
        );

        let (cached, dispatch) = {
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
            let mut prompt_cache_pending = guard
                .prompt_cache_prebaseline_records
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
            let mut prompt_cache_applied_terminal_ids = HashSet::new();
            if let BuiltSubscriptionTopicPayload::Json(payload) = &mut built_payload
                && !prompt_cache_replay.is_empty()
            {
                apply_prompt_cache_records_to_payload(
                    &topic,
                    payload,
                    &prompt_cache_replay,
                    &mut prompt_cache_applied_terminal_ids,
                    prompt_cache_build
                        .as_ref()
                        .map_or(0, |build| build.baseline_row_id),
                )?;
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
            if let Some(existing) = guard.topics.get_mut(&topic_key) {
                if existing.snapshot_frame.payload_bytes.as_ref() == serialized_payload.as_slice()
                    && existing.dirty
                    && existing.dashboard_materializer.is_some()
                    && refreshed_dashboard_materializer.is_some()
                {
                    existing.dirty = false;
                    existing.refresh_scheduled = false;
                    existing.runtime_topic_recovery_retry_at = None;
                    existing.snapshot_built_at = Instant::now();
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
                    return Ok(Some(existing.clone()));
                }
                if reuse_unchanged_cached_topic(existing, &serialized_payload).is_some() {
                    self.dashboard_topology_counters
                        .record_frame_reused(topic.name());
                    if let Some(build) = &prompt_cache_build {
                        existing.prompt_cache_full_hydration_count =
                            existing.prompt_cache_full_hydration_count.saturating_add(1);
                        existing.prompt_cache_baseline_at = Some(Instant::now());
                        existing.prompt_cache_baseline_row_id = build.baseline_row_id;
                        existing.prompt_cache_response_source = "database_reconcile";
                        existing.prompt_cache_applied_terminal_ids =
                            prompt_cache_applied_terminal_ids;
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
                dirty: false,
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
                prompt_cache_refresh_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.prompt_cache_refresh_scheduled),
                prompt_cache_reconcile_scheduled: guard
                    .topics
                    .get(&topic_key)
                    .is_some_and(|entry| entry.prompt_cache_reconcile_scheduled),
                prompt_cache_pending_records: BTreeMap::new(),
                prompt_cache_applied_terminal_ids: if matches!(
                    topic,
                    SubscriptionTopic::PromptCacheWindow { .. }
                        | SubscriptionTopic::PromptCacheStickyWindow { .. }
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
                prompt_cache_reconcile_required: false,
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
            (next, dispatch)
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
                RuntimeMutation::Invocation(_) | RuntimeMutation::AttemptChanged { .. } => {}
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
                if let Err(err) = self
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    self.defer_runtime_topic_recovery_retry(&topic).await;
                    warn!(
                        ?err,
                        topic = %topic.name(),
                        recovery = "dirty_last_good",
                        "bounded runtime mutation recovery retained last-good topic frame"
                    );
                }
            }
            tokio::task::yield_now().await;
        }
    }

    async fn defer_runtime_topic_recovery_retry(&self, topic: &SubscriptionTopic) -> Duration {
        let Ok(topic_key) = topic.cache_key() else {
            return RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF;
        };
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key) {
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

    async fn schedule_prompt_cache_topic_projection(
        &self,
        state: Arc<AppState>,
        mutations: &[SequencedRuntimeMutation],
    ) {
        // The active dependency lookup comes before the compact preview lookup below. Inactive
        // Prompt Cache topics therefore never allocate preview data or read runtime state.
        let active_topic_keys = {
            let guard = self.state.lock().await;
            Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::PromptCacheProjection,
            )
        };
        if active_topic_keys.is_empty() {
            return;
        }

        let mut records = Vec::new();
        let mut reconcile_required = false;
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
                Ok(None) => reconcile_required = true,
                Err(err) => {
                    reconcile_required = true;
                    warn!(?err, "failed to build active prompt cache topic delta");
                }
            }
        }
        if records.is_empty() && !reconcile_required {
            return;
        }

        let (scheduled, reconciles) = {
            let mut guard = self.state.lock().await;
            let mut scheduled = Vec::new();
            let mut reconciles = Vec::new();
            for topic_key in active_topic_keys {
                let Some(topic) = guard.active_topics.get(&topic_key).cloned() else {
                    continue;
                };
                let Some(cached) = guard.topics.get_mut(&topic_key) else {
                    if !records.is_empty() {
                        let pending = guard
                            .prompt_cache_prebaseline_records
                            .entry(topic_key)
                            .or_default();
                        for record in &records {
                            pending.insert(record.identity.clone(), record.clone());
                        }
                    }
                    continue;
                };
                let requires_bounded_reconcile = reconcile_required || cached.dirty;
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
            (scheduled, reconciles)
        };

        for topic in scheduled {
            let hub = state.subscription_hub.clone();
            let state = state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE).await;
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
        if let Some(cached) = self.state.lock().await.topics.get_mut(&topic_key)
            && cached.dirty
        {
            cached.prompt_cache_pressure_deferred = pressure_deferred;
        }
    }

    async fn materialize_prompt_cache_topic(
        &self,
        _state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
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
            let applied = match apply_prompt_cache_records_to_payload(
                topic,
                &mut cached.snapshot_payload,
                &records,
                &mut cached.prompt_cache_applied_terminal_ids,
                cached.prompt_cache_baseline_row_id,
            ) {
                Ok(applied) => applied,
                Err(err) => {
                    for record in records {
                        cached
                            .prompt_cache_pending_records
                            .insert(record.identity.clone(), record);
                    }
                    return Err(err);
                }
            };
            if !applied {
                return Ok(());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let serialized_payload = serde_json::to_vec(&cached.snapshot_payload)?;
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
            SubscriptionDispatchEvent { frame }
        };
        let _ = self.broadcaster.send(dispatch);
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
        let has_active_prompt_cache_window = {
            let guard = self.state.lock().await;
            !Self::active_topic_keys_for_dependency(
                &guard,
                &RuntimeTopicDependency::PromptCacheWindow,
            )
            .is_empty()
        };
        if !has_active_prompt_cache_window {
            return Ok(());
        }
        let binding = serde_json::to_value(
            load_prompt_cache_conversation_binding_response_for_key(
                state.as_ref(),
                prompt_cache_key.to_string(),
            )
            .await?,
        )?;
        let mut dispatches = Vec::new();
        let mut reconciles = Vec::new();
        let mut guard = self.state.lock().await;
        let active_topic_keys = Self::active_topic_keys_for_dependency(
            &guard,
            &RuntimeTopicDependency::PromptCacheWindow,
        );
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
            cached.prompt_cache_bounded_key_hydration_count = cached
                .prompt_cache_bounded_key_hydration_count
                .saturating_add(1);
            let Some(changed) = patch_prompt_cache_binding_payload(
                &mut cached.snapshot_payload,
                prompt_cache_key,
                &binding,
            ) else {
                cached.prompt_cache_reconcile_required = true;
                if !cached.prompt_cache_reconcile_scheduled {
                    cached.prompt_cache_reconcile_scheduled = true;
                    reconciles.push(cached.topic.clone());
                }
                continue;
            };
            if !changed {
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

async fn load_persisted_prompt_cache_identities(
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
        let mut separated = query.separated(" OR ");
        for (invoke_id, occurred_at) in chunk {
            separated
                .push("(invoke_id = ")
                .push_bind(*invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(*occurred_at)
                .push(")");
        }
        separated.push_unseparated(")");
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
                    if let Err(err) = hub.expire_prompt_cache_topic_window(&topic_key).await {
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
    fn uses_server_push_cadence(&self, mode: RuntimeProjectionMode) -> bool {
        self.is_closed_summary_topic()
            || matches!(
                self,
                Self::PromptCacheWindow { .. } | Self::PromptCacheStickyWindow { .. }
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
        match self {
            Self::DashboardWorkingConversationsCurrent { .. } => true,
            Self::ParallelWorkCurrent { range, .. } => range != "yesterday",
            _ => false,
        }
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
            Self::PromptCacheConversationBindingCurrent { scope }
            | Self::PromptCacheConversationOperationsWindow { scope, .. } => {
                vec![RuntimeTopicDependency::Binding(
                    scope.binding_key().to_string(),
                )]
            }
            Self::InvocationPoolAttempts { invoke_id } => {
                vec![RuntimeTopicDependency::Attempt(invoke_id.clone())]
            }
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
            Self::DashboardWorkingConversationsCurrent { .. }
            | Self::InvocationWindow { .. }
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
                Ok(Self::DashboardWorkingConversationsCurrent {
                    page_size: parse_i64_param(
                        params,
                        "pageSize",
                        Some(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE),
                    )?,
                    recent_invocation_limit: parse_i64_param(
                        params,
                        "recentInvocationLimit",
                        Some(SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT),
                    )?,
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
                    | Self::InvocationPoolAttempts { .. } => false,
                }
            }
            RuntimeMutation::AttemptChanged { invoke_id } => matches!(
                self,
                Self::InvocationPoolAttempts { invoke_id: current } if current == invoke_id
            ),
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
    existing.snapshot_built_at = Instant::now();
    Some(existing.clone())
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
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut stream = response.into_body().into_data_stream();
        let mut buffered = Vec::new();
        let mut events = BTreeMap::new();
        while events.len() < expected.len() {
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
            assert_eq!(
                guard.server_push_subscribers.len(),
                0,
                "network topics must not start subscription-owned cadence tasks",
            );
            assert!(
                guard.server_push_tasks.is_empty(),
                "network topics must be driven by the shared projection cadence",
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
        state
            .subscription_hub
            .handle_runtime_mutation_batch(
                state.clone(),
                (1..=10_000)
                    .map(|sequence| SequencedRuntimeMutation {
                        sequence,
                        mutation: RuntimeMutation::invocation(
                            &fallback,
                            RuntimeMutationKind::RuntimeUpsert,
                        ),
                    })
                    .collect(),
            )
            .await;
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
            collect_dashboard_runtime_topology_sse_events(first_response),
            collect_dashboard_runtime_topology_sse_events(second_response),
        );
        state.shutdown.cancel();
        assert_eq!(
            first_frames, second_frames,
            "both SSE owners should receive the same serialized live frames",
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
            assert_eq!(topic.reconnect_churn_count, 1);
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
        ] {
            assert_eq!(topic.generic_fallback_build_count, 0);
            assert_eq!(topic.live_path_db_read_count, 0);
        }
        for topic in [delivery.working_conversations, delivery.parallel_work] {
            assert_eq!(topic.generic_fallback_build_count, 1);
            assert_eq!(topic.live_path_db_read_count, 1);
            assert_eq!(topic.builder_count, 1);
            assert_eq!(topic.materialization_count, 1);
            assert_eq!(topic.serialization_count, 1);
            assert_eq!(topic.cursor_advanced, 1);
        }
        assert!(
            state
                .subscription_hub
                .dashboard_delivery_has_degraded_signal(),
            "generic fallback topics must not be classified as healthy"
        );
    }

    #[test]
    fn dashboard_delivery_reconnect_churn_is_degraded() {
        let counters = DashboardDeliveryTopologyCounters::default();
        counters.record_reconnect_churn("dashboard.working-conversations.current");

        assert!(counters.has_degraded_signal());
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
        assert!(open_parallel.is_unmigrated_dashboard_hot_projection());
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
            prompt_cache_refresh_scheduled: false,
            prompt_cache_reconcile_scheduled: false,
            prompt_cache_pending_records: BTreeMap::new(),
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
        let payload = serde_json::to_vec(&cached.snapshot_payload).expect("cached payload");

        let reused = reuse_unchanged_cached_topic(&mut cached, &payload);

        assert!(reused.is_some());
        assert!(!cached.refresh_scheduled);
        assert!(!cached.prompt_cache_reconcile_required);
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
