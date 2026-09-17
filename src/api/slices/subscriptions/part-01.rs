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
const UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(250);
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
const SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS: usize = 10_000;
const SUMMARY_TERMINAL_OVERLAY_MAX_BYTES: usize = 64 * 1024 * 1024;
const SUMMARY_TERMINAL_OVERLAY_MAX_ACCOUNT_OVERFLOW_MARKERS: usize = 1_024;
const SUMMARY_DELTA_JOURNAL_MAX_GAP_PROOFS: usize = 4_096;
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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) summary_live_tail_reason: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) summary_live_tail_stage: String,
    pub(crate) summary_live_tail_gap_count: u64,
    pub(crate) summary_live_tail_watermark: u64,
    pub(crate) summary_live_tail_epoch: u64,
    pub(crate) summary_live_tail_elapsed_ms: u64,
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
        let mut health = build_hot_topic_health_snapshot(
            counters,
            recovery,
            activity_cadence,
            network_cadence,
            current_and_terminal_cadence,
            working_conversations_cadence,
            parallel_work_cadence,
        );
        health.state = dashboard_hot_topics_health_state(&health);
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

fn hot_topic_health_for_counter(
    counter: DashboardTopicTopologyCounterSnapshot,
    cadence_miss_count: u64,
    recovery: DashboardHotTopicRecoveryState,
) -> DashboardHotTopicHealthSnapshot {
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
        ..DashboardHotTopicHealthSnapshot::default()
    }
}

fn build_hot_topic_health_snapshot(
    counters: DashboardDeliveryTopologyCounterSnapshot,
    recovery: DashboardHotTopicRecoveryHealth,
    activity_cadence: u64,
    network_cadence: u64,
    current_and_terminal_cadence: u64,
    working_conversations_cadence: u64,
    parallel_work_cadence: u64,
) -> DashboardHotTopicsHealthSnapshot {
    DashboardHotTopicsHealthSnapshot {
        state: String::new(),
        activity: hot_topic_health_for_counter(
            counters.activity,
            activity_cadence,
            recovery.activity,
        ),
        summary: hot_topic_health_for_counter(
            counters.summary,
            current_and_terminal_cadence,
            recovery.summary,
        ),
        network_timeseries: hot_topic_health_for_counter(
            counters.network_timeseries,
            network_cadence,
            recovery.network_timeseries,
        ),
        network_recent: hot_topic_health_for_counter(
            counters.network_recent,
            network_cadence,
            recovery.network_recent,
        ),
        working_conversations: hot_topic_health_for_counter(
            counters.working_conversations,
            working_conversations_cadence,
            recovery.working_conversations,
        ),
        parallel_work: hot_topic_health_for_counter(
            counters.parallel_work,
            parallel_work_cadence,
            recovery.parallel_work,
        ),
        timeseries: hot_topic_health_for_counter(
            counters.timeseries,
            current_and_terminal_cadence,
            recovery.timeseries,
        ),
    }
}

fn dashboard_hot_topics_health_state(health: &DashboardHotTopicsHealthSnapshot) -> String {
    let states = [
        health.activity.state.as_str(),
        health.summary.state.as_str(),
        health.network_timeseries.state.as_str(),
        health.network_recent.state.as_str(),
        health.working_conversations.state.as_str(),
        health.parallel_work.state.as_str(),
        health.timeseries.state.as_str(),
    ];
    if states.contains(&"degraded") {
        "degraded".to_string()
    } else if states.contains(&"deferred") {
        "deferred".to_string()
    } else {
        "healthy".to_string()
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
    // Summary projection hydration is single-flight per service instance. Keeping this with the
    // hub avoids suppressing bootstrap for an independent AppState (including test fixtures).
    summary_projection_refresh: tokio::sync::Mutex<()>,
    // Historical coverage recovery has independent lifetime and database admission from the
    // rolling Projection refresh. It still needs one owner so a cadence tick cannot duplicate a
    // page already being reduced by the recovery worker.
    summary_coverage_recovery: tokio::sync::Mutex<()>,
    broadcaster: broadcast::Sender<SubscriptionDispatchEvent>,
    runtime_mutation_bus: Arc<RuntimeMutationBus>,
    runtime_topic_recovery_notify: Arc<Notify>,
    internal_broadcast_listener_count: AtomicUsize,
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
    summary_snapshots: HashMap<SummarySnapshotKey, SummarySnapshotEntry>,
    summary_projection: Option<Arc<SummaryProjection>>,
    // Terminal slices are ephemeral delivery batches. The Summary-owned journal only receives
    // entries after the writer's SQLite commit ACK, so it is a bounded exact delta between an
    // immutable base projection and the latest rolling view.
    summary_delta_journal: SummaryDeltaJournal,
    // SQLite source descriptors are consumed once per process lifetime. A restart starts at
    // zero and reconstructs the bounded durable tail before any RollingDelta refresh.
    summary_source_change_cursor: u64,
    summary_live_tail_readiness: SummaryLiveTailReadiness,
    // All-time coverage can lag independently of rolling coverage. Keep its replay budget
    // separate so an all-time archive gap cannot make healthy rolling topics unavailable.
    summary_terminal_overlay_all_time: VecDeque<DashboardActivityTerminalDelta>,
    summary_terminal_overlay_all_time_bytes: usize,
    // A shared all-time queue can overflow on one account while another account remains
    // serviceable. Retain the dropped-through proof per account so a global rebuild cannot
    // silently clear an account's fail-closed marker.
    summary_terminal_overlay_all_time_overflowed_through_account: HashMap<i64, u64>,
    // Once the account marker cap is reached, retain one bounded proof for all further account
    // keys. Account reads remain fail-closed unless their own projection proves this watermark.
    summary_terminal_overlay_all_time_overflowed_through_unknown_account: Option<u64>,
    summary_terminal_overlay_all_time_overflowed_through_sequence: Option<u64>,
    summary_http_interest_at: Option<Instant>,
    summary_http_all_time_interest_at: Option<Instant>,
    summary_projection_revision: u64,
    prompt_cache_prebaseline_records: HashMap<String, BTreeMap<String, PromptCacheTopicDelta>>,
    prompt_cache_prebaseline_key_hydrations: HashMap<String, BTreeSet<String>>,
    parallel_work_prebaseline_mutations:
        HashMap<String, BTreeMap<String, RuntimeInvocationMutation>>,
    runtime_topic_recovery_generation: u64,
    runtime_topic_recovery_queue: VecDeque<(String, u64)>,
    runtime_topic_recovery_queued: HashSet<String>,
    runtime_topic_recovery_running: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryLiveTailReadiness {
    pub(crate) reason: String,
    pub(crate) stage: String,
    pub(crate) gap_count: u64,
    pub(crate) watermark: u64,
    pub(crate) epoch: u64,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct SummaryDeltaCursor(pub(crate) u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SummarySourceIdentity {
    pub(crate) row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl SummarySourceIdentity {
    fn from_delta(delta: &DashboardActivityTerminalDelta) -> Option<Self> {
        delta.persisted_row_id.map(|row_id| Self {
            row_id,
            invoke_id: delta.invoke_id.clone(),
            occurred_at: delta.occurred_at.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryDeltaEntry {
    pub(crate) cursor: SummaryDeltaCursor,
    pub(crate) delta: DashboardActivityTerminalDelta,
}

// A dropped, conflicting, or out-of-order journal entry never permits a stale global response.
// The reconciliation path consumes this proof to restrict only selections that could include it.
#[derive(Debug, Clone)]
pub(crate) struct DeltaGapProof {
    pub(crate) cursor: SummaryDeltaCursor,
    // A journal cursor and a terminal sequence are different domains. `None` means this proof
    // was created by source-journal compaction and must not be retired by a terminal watermark.
    pub(crate) terminal_sequence: Option<u64>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) occurred_at: String,
    pub(crate) row_id: Option<i64>,
    pub(crate) invoke_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryRollingDeltaSnapshot {
    pub(crate) projection: Arc<SummaryProjection>,
    pub(crate) entries: Vec<DashboardActivityTerminalDelta>,
    pub(crate) gaps: Vec<DeltaGapProof>,
}
