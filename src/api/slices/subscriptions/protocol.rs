use super::*;

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
pub(crate) struct SubscriptionCompactResumeCursor {
    pub(crate) topic_index: usize,
    pub(crate) cursor: u64,
    pub(crate) schema_epoch: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum SubscriptionResumeCursorQuery {
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
    Unavailable {
        topic: SubscriptionTopicDescriptor,
        #[serde(rename = "topicKey")]
        topic_key: String,
        #[serde(rename = "schemaEpoch")]
        schema_epoch: String,
        #[serde(rename = "errorCode")]
        error_code: &'static str,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopicFrameKind {
    Snapshot,
    Replay,
    Live,
    Unavailable,
}

#[derive(Debug)]
pub(crate) struct SerializedTopicFrame {
    pub(crate) topic_key: String,
    pub(crate) schema_epoch: String,
    pub(crate) cursor: u64,
    pub(crate) descriptor: SubscriptionTopicDescriptor,
    pub(crate) fingerprint: u64,
    pub(crate) payload_bytes: Bytes,
    pub(crate) envelope_metadata_bytes: Bytes,
}

impl SerializedTopicFrame {
    pub(crate) fn event_chunks(&self, kind: TopicFrameKind) -> [Bytes; 4] {
        let prefix = match kind {
            TopicFrameKind::Snapshot => Bytes::from_static(b"data: {\"type\":\"snapshot"),
            TopicFrameKind::Replay => Bytes::from_static(b"data: {\"type\":\"replay"),
            TopicFrameKind::Live => Bytes::from_static(b"data: {\"type\":\"live"),
            TopicFrameKind::Unavailable => Bytes::from_static(b"data: {\"type\":\"unavailable"),
        };
        [
            prefix,
            self.envelope_metadata_bytes.clone(),
            self.payload_bytes.clone(),
            Bytes::from_static(b"}\n\n"),
        ]
    }

    pub(crate) fn retained_bytes(&self) -> usize {
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
pub(crate) enum SubscriptionTopicClass {
    HotProjection,
    ClosedSnapshot,
    BoundedColdHydrate,
}

impl SubscriptionTopicClass {
    pub(crate) fn as_str(self) -> &'static str {
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
pub(crate) struct DashboardHotTopicRecoveryState {
    pub(crate) degraded: bool,
    pub(crate) deferred: bool,
}

impl DashboardHotTopicRecoveryState {
    pub(crate) fn merge(&mut self, other: Self) {
        self.degraded |= other.degraded;
        self.deferred |= other.deferred;
    }
}

#[derive(Debug, Default)]
pub(crate) struct DashboardHotTopicRecoveryHealth {
    pub(crate) activity: DashboardHotTopicRecoveryState,
    pub(crate) summary: DashboardHotTopicRecoveryState,
    pub(crate) network_timeseries: DashboardHotTopicRecoveryState,
    pub(crate) network_recent: DashboardHotTopicRecoveryState,
    pub(crate) working_conversations: DashboardHotTopicRecoveryState,
    pub(crate) parallel_work: DashboardHotTopicRecoveryState,
    pub(crate) timeseries: DashboardHotTopicRecoveryState,
}

impl DashboardHotTopicRecoveryHealth {
    pub(crate) fn record(
        &mut self,
        topic: &SubscriptionTopic,
        state: DashboardHotTopicRecoveryState,
    ) {
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
pub(crate) struct DashboardTopicTopologyCounters {
    pub(crate) active_subscriber_count: AtomicU64,
    pub(crate) builder_count: AtomicU64,
    pub(crate) generic_fallback_build_count: AtomicU64,
    pub(crate) live_path_db_read_count: AtomicU64,
    pub(crate) materialization_count: AtomicU64,
    pub(crate) serialization_count: AtomicU64,
    pub(crate) payload_clone_count: AtomicU64,
    pub(crate) frame_bytes_count: AtomicU64,
    pub(crate) frame_reused: AtomicU64,
    pub(crate) cursor_advanced: AtomicU64,
    pub(crate) lagged_count: AtomicU64,
    pub(crate) skipped_count: AtomicU64,
    pub(crate) reconnect_churn_count: AtomicU64,
    pub(crate) business_payload_count: AtomicU64,
    pub(crate) json_overlay_count: AtomicU64,
}

impl DashboardTopicTopologyCounters {
    pub(crate) fn snapshot(&self) -> DashboardTopicTopologyCounterSnapshot {
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
    pub(crate) fn reset(&self) {
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
pub(crate) struct DashboardDeliveryTopologyCounters {
    pub(crate) activity: DashboardTopicTopologyCounters,
    pub(crate) summary: DashboardTopicTopologyCounters,
    pub(crate) network_timeseries: DashboardTopicTopologyCounters,
    pub(crate) network_recent: DashboardTopicTopologyCounters,
    pub(crate) working_conversations: DashboardTopicTopologyCounters,
    pub(crate) parallel_work: DashboardTopicTopologyCounters,
    pub(crate) timeseries: DashboardTopicTopologyCounters,
    pub(crate) working_conversations_cadence_miss_count: AtomicU64,
    pub(crate) parallel_work_cadence_miss_count: AtomicU64,
}

impl DashboardDeliveryTopologyCounters {
    pub(crate) fn for_topic(&self, topic_name: &str) -> Option<&DashboardTopicTopologyCounters> {
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

    pub(crate) fn set_active_subscriber_count(&self, topic_name: &str, count: usize) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic
                .active_subscriber_count
                .store(count as u64, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_materialization(&self, topic_name: &str, generic_fallback: bool) {
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

    pub(crate) fn record_serialization(&self, topic_name: &str, frame_bytes: usize) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.serialization_count.fetch_add(1, Ordering::Relaxed);
            topic
                .frame_bytes_count
                .fetch_add(frame_bytes as u64, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_frame_reused(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.frame_reused.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_shared_frame_delivery(&self, topic_name: &str) {
        let Some(topic) = self.for_topic(topic_name) else {
            return;
        };
        if topic.active_subscriber_count.load(Ordering::Relaxed) > 1 {
            topic.frame_reused.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_cursor_advanced(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.cursor_advanced.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_lag(&self, topic_name: &str, skipped: u64) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.lagged_count.fetch_add(1, Ordering::Relaxed);
            topic.skipped_count.fetch_add(skipped, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_reconnect_churn(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.reconnect_churn_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_business_payload(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.business_payload_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_json_overlay(&self, topic_name: &str) {
        if let Some(topic) = self.for_topic(topic_name) {
            topic.json_overlay_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_cadence_miss(&self, topic_name: &str) {
        let counter = match topic_name {
            "dashboard.working-conversations.current" => {
                &self.working_conversations_cadence_miss_count
            }
            "stats.parallel-work.current" => &self.parallel_work_cadence_miss_count,
            _ => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> DashboardDeliveryTopologyCounterSnapshot {
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

    pub(crate) fn hot_topic_health(
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
                ..DashboardHotTopicHealthSnapshot::default()
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

    pub(crate) fn has_degraded_signal(&self) -> bool {
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
    pub(crate) fn reset(&self) {
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
