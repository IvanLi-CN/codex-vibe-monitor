use super::*;
use serde::de::DeserializeOwned;
use serde_json::json;

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

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionDispatchEvent {
    pub(crate) topic_key: String,
    pub(crate) schema_epoch: String,
    pub(crate) cursor: u64,
    pub(crate) payload: Value,
    pub(crate) descriptor: SubscriptionTopicDescriptor,
}

#[derive(Debug)]
pub(crate) struct SubscriptionHub {
    state: Mutex<SubscriptionHubState>,
    broadcaster: broadcast::Sender<SubscriptionDispatchEvent>,
}

#[derive(Debug, Default)]
struct SubscriptionHubState {
    topics: HashMap<String, CachedSubscriptionTopic>,
    active_subscribers: HashMap<String, usize>,
    active_topic_names: HashMap<String, usize>,
    dashboard_live_subscriber_count: usize,
    server_push_subscribers: HashMap<String, usize>,
    server_push_tasks: HashSet<String>,
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
    summary_refresh_scheduled: bool,
    summary_refresh_in_flight: bool,
    summary_pending_event_count: u64,
    summary_retry_backoff_ms: u64,
    latest_live_snapshot: Option<DashboardActivityLiveSnapshot>,
    calendar_anchor: Option<String>,
    continuity_reset_cursor: Option<u64>,
    snapshot_payload: Value,
    snapshot_bytes: usize,
    replay_events: VecDeque<ReplayableTopicEvent>,
    replay_bytes: usize,
}

#[derive(Debug, Clone)]
struct ReplayableTopicEvent {
    cursor: u64,
    payload: Value,
    bytes: usize,
    emitted_at: DateTime<Utc>,
}

struct ServerPushTopicLease {
    hub: Arc<SubscriptionHub>,
    topic_keys: Vec<String>,
}

pub(crate) struct TopicSubscriptionLease {
    hub: Arc<SubscriptionHub>,
    topic_keys: Vec<String>,
    topic_names: Vec<String>,
    dashboard_live_topic_count: usize,
}

impl Drop for TopicSubscriptionLease {
    fn drop(&mut self) {
        if self.topic_keys.is_empty() {
            return;
        }
        let hub = self.hub.clone();
        let topic_keys = std::mem::take(&mut self.topic_keys);
        let topic_names = std::mem::take(&mut self.topic_names);
        let dashboard_live_topic_count = self.dashboard_live_topic_count;
        tokio::spawn(async move {
            hub.release_topic_subscribers(topic_keys, topic_names, dashboard_live_topic_count)
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
    pub(crate) initial: Vec<SubscriptionEventEnvelope>,
    pub(crate) last_sent_cursors: HashMap<String, u64>,
    pub(crate) outcomes: Vec<TopicInitOutcome>,
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
    pub(crate) fn new() -> Self {
        let (broadcaster, _) = broadcast::channel(1_024);
        Self {
            state: Mutex::new(SubscriptionHubState::default()),
            broadcaster,
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SubscriptionDispatchEvent> {
        self.broadcaster.subscribe()
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

    async fn register_topic_subscribers(
        self: &Arc<Self>,
        topics: &[SubscriptionTopic],
    ) -> Result<TopicSubscriptionLease, ApiError> {
        let dashboard_live_topic_count = topics
            .iter()
            .filter(|topic| {
                topic.uses_dashboard_activity_live_overlay()
                    || topic.uses_summary_live_overlay()
                    || topic.uses_dashboard_network_live_snapshot()
            })
            .count();
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
        for topic_key in &topic_keys {
            *guard
                .active_subscribers
                .entry(topic_key.clone())
                .or_insert(0) += 1;
        }
        for topic_name in &topic_names {
            *guard
                .active_topic_names
                .entry(topic_name.clone())
                .or_insert(0) += 1;
        }
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_add(dashboard_live_topic_count);
        Ok(TopicSubscriptionLease {
            hub: self.clone(),
            topic_keys,
            topic_names,
            dashboard_live_topic_count,
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
        let dashboard_live_topic_count = usize::from(topic_name == "dashboard.activity.current");
        guard.dashboard_live_subscriber_count = guard
            .dashboard_live_subscriber_count
            .saturating_add(dashboard_live_topic_count);
        let topic_keys = guard
            .topics
            .iter()
            .filter(|(_, cached)| cached.topic.name() == topic_name)
            .map(|(topic_key, _)| topic_key.clone())
            .collect::<Vec<_>>();
        for topic_key in &topic_keys {
            *guard
                .active_subscribers
                .entry(topic_key.clone())
                .or_insert(0) += 1;
        }
        TopicSubscriptionLease {
            hub: self.clone(),
            topic_keys,
            topic_names: vec![topic_name.to_string()],
            dashboard_live_topic_count,
        }
    }

    async fn release_topic_subscribers(
        &self,
        topic_keys: Vec<String>,
        topic_names: Vec<String>,
        dashboard_live_topic_count: usize,
    ) {
        let mut guard = self.state.lock().await;
        for topic_key in topic_keys {
            match guard.active_subscribers.get_mut(&topic_key) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    guard.active_subscribers.remove(&topic_key);
                }
                None => {}
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
            .saturating_sub(dashboard_live_topic_count);
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
                        initial.push(SubscriptionEventEnvelope::Replay {
                            topic: cached.descriptor.clone(),
                            topic_key: topic_key.clone(),
                            schema_epoch: cached.schema_epoch.clone(),
                            cursor: event.cursor,
                            payload: event.payload,
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
                    initial.push(SubscriptionEventEnvelope::Snapshot {
                        topic: cached.descriptor.clone(),
                        topic_key: topic_key.clone(),
                        schema_epoch: cached.schema_epoch.clone(),
                        cursor: cached.cursor,
                        payload: cached.snapshot_payload.clone(),
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
                    initial.push(SubscriptionEventEnvelope::Snapshot {
                        topic: cached.descriptor.clone(),
                        topic_key: topic_key.clone(),
                        schema_epoch: cached.schema_epoch.clone(),
                        cursor: cached.cursor,
                        payload: cached.snapshot_payload.clone(),
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
            if event.cursor <= resume.cursor {
                matched = true;
                continue;
            }
            if !matched
                && resume.cursor > 0
                && event.cursor > resume.cursor
                && cached
                    .replay_events
                    .front()
                    .is_some_and(|front| front.cursor > resume.cursor)
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
                .is_some_and(|front| front.cursor > resume.cursor)
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
        if let Some(existing) = self.state.lock().await.topics.get(&topic_key).cloned()
            && !existing.dirty
            && (!topic.is_closed_summary_topic()
                || existing.calendar_anchor == subscription_calendar_anchor(&topic))
        {
            return Ok(existing);
        }
        self.refresh_topic(state, topic, false).await
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
        let mut payload = topic.build_payload(state.clone()).await?;
        let mut payload_bytes = serialized_len(&payload)?;

        let (cached, dispatch) = {
            let mut guard = self.state.lock().await;
            if require_active_owner
                && guard
                    .active_subscribers
                    .get(&topic_key)
                    .copied()
                    .unwrap_or_default()
                    == 0
            {
                if let Some(cached) = guard.topics.get_mut(&topic_key) {
                    cached.dirty = true;
                    cached.refresh_scheduled = false;
                    cached.latest_live_snapshot = None;
                }
                return Ok(None);
            }
            if let Some(live) = guard
                .topics
                .get(&topic_key)
                .and_then(|cached| cached.latest_live_snapshot.as_ref())
                .cloned()
            {
                apply_topic_live_overlay_to_payload(state.as_ref(), &topic, &mut payload, &live)?;
                payload_bytes = serialized_len(&payload)?;
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
                latest_live_snapshot: guard
                    .topics
                    .get(&topic_key)
                    .and_then(|entry| entry.latest_live_snapshot.clone()),
                calendar_anchor: subscription_calendar_anchor(&topic),
                continuity_reset_cursor,
                snapshot_payload: payload.clone(),
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
                let replay_event = ReplayableTopicEvent {
                    cursor: next.cursor,
                    payload: payload.clone(),
                    bytes: payload_bytes,
                    emitted_at: Utc::now(),
                };
                next.replay_events.push_back(replay_event);
                next.replay_bytes = next.replay_bytes.saturating_add(payload_bytes);
                prune_replay_window(&mut next.replay_events, &mut next.replay_bytes);
            }
            guard.topics.insert(topic_key.clone(), next.clone());
            let dispatch = emit_live.then(|| SubscriptionDispatchEvent {
                topic_key: topic_key.clone(),
                schema_epoch: schema_epoch.clone(),
                cursor: next.cursor,
                payload: payload.clone(),
                descriptor: descriptor.clone(),
            });
            (next, dispatch)
        };

        tracing::debug!(
            topic_key,
            schema_epoch,
            emit_live,
            snapshot_build_ms = started.elapsed().as_millis() as u64,
            payload_bytes,
            "subscription topic snapshot built"
        );

        if let Some(dispatch) = dispatch {
            let _ = self.broadcaster.send(dispatch.clone());
            tracing::debug!(
                topic_key = dispatch.topic_key,
                cursor = dispatch.cursor,
                fanout_receivers = self.broadcaster.receiver_count(),
                "subscription topic live event dispatched"
            );
        }

        Ok(Some(cached))
    }

    pub(crate) async fn handle_internal_broadcast(
        &self,
        state: Arc<AppState>,
        payload: BroadcastPayload,
    ) {
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
                    if matches!(payload, BroadcastPayload::DashboardActivityLive { .. })
                        && (cached.topic.uses_summary_live_overlay()
                            || cached.topic.uses_dashboard_activity_live_overlay())
                        && let BroadcastPayload::DashboardActivityLive { snapshot } = &payload
                    {
                        cached.latest_live_snapshot = Some(snapshot.as_ref().clone());
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

            if cached.topic.uses_summary_topic_refresh()
                && let BroadcastPayload::Records { records } = &payload
            {
                if records
                    .iter()
                    .any(crate::app_state::runtime_store_record_is_terminal)
                    && let Err(err) = self
                        .schedule_summary_topic_refresh(
                            state.clone(),
                            cached.topic.clone(),
                            records.len() as u64,
                        )
                        .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to schedule summary topic refresh"
                    );
                }
                continue;
            }

            if cached.topic.uses_conversation_overview_refresh()
                && matches!(
                    payload,
                    BroadcastPayload::Records { .. }
                        | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. }
                )
            {
                if let Err(err) = self
                    .schedule_conversation_overview_topic_refresh(
                        state.clone(),
                        cached.topic.clone(),
                    )
                    .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to schedule conversation overview topic refresh"
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

            if matches!(&payload, BroadcastPayload::Records { .. })
                && cached.topic.uses_dashboard_activity_live_overlay()
            {
                let needs_refresh = match &payload {
                    BroadcastPayload::Records { records } => records
                        .iter()
                        .any(crate::app_state::runtime_store_record_is_terminal),
                    _ => false,
                };
                if needs_refresh
                    && let Err(err) = self
                        .schedule_dashboard_activity_topic_refresh(
                            state.clone(),
                            cached.topic.clone(),
                        )
                        .await
                {
                    warn!(
                        ?err,
                        topic = %cached.topic.name(),
                        "failed to schedule dashboard activity topic refresh"
                    );
                }
                continue;
            }

            if let Err(err) = self
                .refresh_topic(state.clone(), cached.topic.clone(), true)
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
            let payload_bytes = serialized_len(&payload)?;
            cached.cursor = cached.cursor.saturating_add(1);
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                cursor: cached.cursor,
                payload: payload.clone(),
                bytes: payload_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(payload_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent {
                topic_key: topic_key.clone(),
                schema_epoch: cached.schema_epoch.clone(),
                cursor: cached.cursor,
                payload,
                descriptor: cached.descriptor.clone(),
            }
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

            let payload_bytes = serialized_len(&payload)?;
            cached.cursor = cached.cursor.saturating_add(1);
            cached.snapshot_payload = payload.clone();
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                cursor: cached.cursor,
                payload: payload.clone(),
                bytes: payload_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(payload_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);

            SubscriptionDispatchEvent {
                topic_key: topic_key.clone(),
                schema_epoch: cached.schema_epoch.clone(),
                cursor: cached.cursor,
                payload,
                descriptor: cached.descriptor.clone(),
            }
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

async fn run_server_push_topic_loop(
    hub: Arc<SubscriptionHub>,
    state: Arc<AppState>,
    topic_key: String,
    topic: SubscriptionTopic,
) {
    if topic.is_closed_summary_topic() {
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
                    if let Err(err) = hub.refresh_topic(state.clone(), topic.clone(), true).await {
                        warn!(?err, topic = %topic.name(), "failed to refresh closed summary topic at calendar rollover");
                    }
                }
            }
        }
        return;
    }

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
                if let Err(err) = hub.refresh_topic(state.clone(), topic.clone(), true).await {
                    warn!(?err, topic = %topic.name(), "failed to push subscription topic cadence");
                }
            }
        }
    }
}

pub(crate) fn spawn_subscription_broadcast_listener(state: Arc<AppState>) {
    let hub = state.subscription_hub.clone();
    let shutdown = state.shutdown.clone();
    let mut receiver = state.broadcaster.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                item = receiver.recv() => {
                    match item {
                        Ok(payload) => hub.handle_internal_broadcast(state.clone(), payload).await,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(skipped, "subscription mutation listener lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        }
    });
}

pub(crate) async fn topic_sse_stream(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubscriptionStreamQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
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
    let server_push_topics = selected_topics
        .iter()
        .filter(|topic| topic.uses_server_push_cadence())
        .cloned()
        .collect::<Vec<_>>();
    let server_push_lease = state
        .subscription_hub
        .register_server_push_topics(state.clone(), server_push_topics)
        .await?;

    let initial_stream = stream::iter(
        initial
            .into_iter()
            .filter_map(|payload| serialize_sse_event(&payload).ok()),
    );

    let live_stream = async_stream::stream! {
        let _topic_lease = topic_lease;
        let _server_push_lease = server_push_lease;
        let mut last_seen = last_seen_by_topic;
        loop {
            match live_receiver.recv().await {
                Ok(dispatch) => {
                    if !selected_topic_keys.contains(&dispatch.topic_key) {
                        continue;
                    }
                    let previous_cursor = last_seen.get(&dispatch.topic_key).copied().unwrap_or(0);
                    if dispatch.cursor <= previous_cursor {
                        continue;
                    }
                    last_seen.insert(dispatch.topic_key.clone(), dispatch.cursor);
                    let payload = SubscriptionEventEnvelope::Live {
                        topic: dispatch.descriptor.clone(),
                        topic_key: dispatch.topic_key.clone(),
                        schema_epoch: dispatch.schema_epoch.clone(),
                        cursor: dispatch.cursor,
                        payload: dispatch.payload.clone(),
                    };
                    if let Ok(event) = serialize_sse_event(&payload) {
                        yield event;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(skipped, "subscription live fanout lagged");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    let merged = initial_stream.chain(live_stream);
    Ok(Sse::new(merged).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

impl SubscriptionTopic {
    fn uses_server_push_cadence(&self) -> bool {
        matches!(self, Self::DashboardNetworkRecentCurrent) || self.is_closed_summary_topic()
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

    fn uses_summary_topic_refresh(&self) -> bool {
        self.uses_summary_live_overlay()
    }

    fn uses_conversation_overview_refresh(&self) -> bool {
        matches!(self, Self::InvocationHistoryOverview { .. })
    }

    fn uses_dashboard_network_live_snapshot(&self) -> bool {
        matches!(self, Self::DashboardNetworkTimeseriesWindow { .. })
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

    fn is_affected_by(&self, payload: &BroadcastPayload) -> bool {
        if self.is_closed_summary_topic()
            && matches!(
                payload,
                BroadcastPayload::Records { .. } | BroadcastPayload::DashboardActivityLive { .. }
            )
        {
            return false;
        }

        match payload {
            BroadcastPayload::Records { records } => match self {
                Self::InvocationHistoryWindow { scope }
                | Self::InvocationHistoryOverview { scope } => {
                    records.iter().any(|record| scope.matches_record(record))
                }
                Self::DashboardActivityCurrent { .. }
                | Self::DashboardNetworkTimeseriesWindow { .. }
                | Self::DashboardNetworkRecentCurrent
                | Self::DashboardWorkingConversationsCurrent { .. }
                | Self::InvocationWindow { .. }
                | Self::PromptCacheWindow { .. }
                | Self::PromptCacheStickyWindow { .. }
                | Self::SummaryCurrent { .. }
                | Self::TimeseriesOpenWindow { .. }
                | Self::ParallelWorkCurrent { .. }
                | Self::ForwardProxyLive => true,
                _ => false,
            },
            BroadcastPayload::PromptCacheConversationChanged { prompt_cache_key } => match self {
                Self::PromptCacheConversationBindingCurrent { scope }
                | Self::PromptCacheConversationOperationsWindow { scope, .. } => {
                    scope.binding_key() == prompt_cache_key
                }
                _ => false,
            },
            BroadcastPayload::PromptCacheConversationStickyRouteChanged {
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
            BroadcastPayload::DashboardActivityLive { .. } => {
                matches!(
                    self,
                    Self::DashboardActivityCurrent { .. }
                        | Self::DashboardNetworkTimeseriesWindow { .. }
                        | Self::DashboardNetworkRecentCurrent
                        | Self::SummaryCurrent { .. }
                )
            }
            BroadcastPayload::PoolAttempts { invoke_id, .. } => matches!(
                self,
                Self::InvocationPoolAttempts { invoke_id: current } if current == invoke_id
            ),
            BroadcastPayload::Quota { .. } => matches!(self, Self::QuotaCurrent),
            BroadcastPayload::Version { .. } => matches!(self, Self::AppVersion),
        }
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

fn serialize_sse_event(
    payload: &SubscriptionEventEnvelope,
) -> Result<Result<Event, Infallible>, ApiError> {
    Event::default()
        .json_data(payload)
        .map(Ok)
        .map_err(ApiError::from)
}

fn serialized_len(payload: &Value) -> Result<usize, ApiError> {
    Ok(serde_json::to_vec(payload)?.len())
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
            .register_topic_subscribers(std::slice::from_ref(&topic))
            .await
            .expect("register first dashboard owner");
        assert!(
            state
                .subscription_hub
                .has_active_dashboard_activity_live_topic()
                .await
        );
        let prepared = state
            .subscription_hub
            .prepare_connection(state.clone(), vec![topic.descriptor()], Vec::new())
            .await
            .expect("prepare first dashboard owner connection");
        assert!(!prepared.initial.is_empty());
        ensure_dashboard_activity_live_snapshot_producer(state.as_ref());
    }

    #[tokio::test]
    async fn inactive_topics_are_marked_dirty_and_rebuilt_on_reconnect() {
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
        assert_eq!(prepared.initial.len(), 1);
        assert_eq!(
            prepared.outcomes[0].disposition,
            TopicInitDisposition::SnapshotResumeMiss
        );
        assert_eq!(prepared.outcomes[0].miss_reason, Some("continuity_reset"));
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
        hub.state.lock().await.topics.insert(topic_key, cached);
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
        assert_eq!(dispatch.payload["inProgressConversationCount"], json!(2));
        assert_eq!(
            dispatch.payload["inProgressRetryConversationCount"],
            json!(1)
        );
        assert_eq!(dispatch.payload["inProgressAvgWaitMs"], json!(40.0));

        hub.apply_summary_live_overlay(&topic, live)
            .await
            .expect("reapply summary live overlay");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), receiver.recv())
                .await
                .is_err()
        );
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

            assert!(topic.uses_server_push_cadence());
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

    fn seeded_cached_topic(
        topic: SubscriptionTopic,
        cursors: &[u64],
        emitted_at: DateTime<Utc>,
    ) -> CachedSubscriptionTopic {
        let descriptor = topic.descriptor();
        let schema_epoch = topic.schema_epoch();
        let replay_events = cursors
            .iter()
            .map(|cursor| ReplayableTopicEvent {
                cursor: *cursor,
                payload: json!({ "cursor": cursor }),
                bytes: 32,
                emitted_at,
            })
            .collect::<VecDeque<_>>();
        let replay_bytes = replay_events.iter().map(|event| event.bytes).sum::<usize>();
        let cursor = cursors.last().copied().unwrap_or(0);

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
            summary_refresh_scheduled: false,
            summary_refresh_in_flight: false,
            summary_pending_event_count: 0,
            summary_retry_backoff_ms: 0,
            latest_live_snapshot: None,
            calendar_anchor: None,
            continuity_reset_cursor: None,
            snapshot_payload: json!({ "cursor": cursor }),
            snapshot_bytes: 32,
            replay_events,
            replay_bytes,
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
        assert!(topic.is_affected_by(&BroadcastPayload::Records {
            records: Vec::new()
        }));
        assert!(
            topic.is_affected_by(&BroadcastPayload::DashboardActivityLive {
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
    fn conversation_configuration_events_only_refresh_binding_and_operations_topics() {
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
        let event = BroadcastPayload::PromptCacheConversationChanged {
            prompt_cache_key: "pck-1".to_string(),
        };

        assert!(!calls.is_affected_by(&event));
        assert!(!overview.is_affected_by(&event));
        assert!(binding.is_affected_by(&event));
        assert!(operations.is_affected_by(&event));
        assert!(
            !binding.is_affected_by(&BroadcastPayload::PromptCacheConversationChanged {
                prompt_cache_key: "pck-2".to_string(),
            })
        );
        assert!(!binding.is_affected_by(&BroadcastPayload::Records {
            records: Vec::new(),
        }));
        assert!(!operations.is_affected_by(&BroadcastPayload::Records {
            records: Vec::new(),
        }));
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
        let event = BroadcastPayload::PromptCacheConversationStickyRouteChanged {
            sticky_key: "sticky-1".to_string(),
            previous_upstream_account_id: 41,
            upstream_account_id: 42,
        };

        assert!(topic_for(41).is_affected_by(&event));
        assert!(topic_for(42).is_affected_by(&event));
        assert!(!topic_for(43).is_affected_by(&event));
        assert!(prompt_cache_topic.is_affected_by(&event));
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

    #[tokio::test]
    async fn dashboard_network_recent_topic_push_cadence_emits_live_payload() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::DashboardNetworkRecentCurrent;
        let descriptor = topic.descriptor();
        let mut receiver = hub.subscribe();

        let prepared = hub
            .prepare_connection(state.clone(), vec![descriptor.clone()], Vec::new())
            .await
            .expect("prepare recent network topic");

        assert_eq!(prepared.initial.len(), 1);
        assert!(topic.uses_server_push_cadence());

        let _lease = hub
            .register_server_push_topics(state, vec![topic])
            .await
            .expect("register recent network push topic");

        let dispatch = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("recent network push should be emitted")
            .expect("recent network dispatch");

        assert_eq!(dispatch.descriptor, descriptor);
        assert_eq!(dispatch.schema_epoch, "dashboard.network-recent.current/v1");
        assert_eq!(
            dispatch
                .payload
                .get("windowSeconds")
                .and_then(Value::as_i64),
            Some(300)
        );
        assert_eq!(
            dispatch
                .payload
                .get("sampleSeconds")
                .and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            dispatch
                .payload
                .get("points")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(300)
        );
    }

    #[tokio::test]
    async fn dashboard_network_recent_topic_push_cadence_is_shared_across_subscribers() {
        let state =
            crate::tests::test_state_with_openai_base(Url::parse("http://127.0.0.1:9").unwrap())
                .await;
        let hub = Arc::new(SubscriptionHub::new());
        let topic = SubscriptionTopic::DashboardNetworkRecentCurrent;
        let topic_key = topic.cache_key().expect("recent topic key");
        let descriptor = topic.descriptor();

        hub.prepare_connection(state.clone(), vec![descriptor.clone()], Vec::new())
            .await
            .expect("prepare first recent network topic connection");
        let first_lease = hub
            .register_server_push_topics(state.clone(), vec![topic.clone()])
            .await
            .expect("register first recent network push topic");
        hub.prepare_connection(state.clone(), vec![descriptor], Vec::new())
            .await
            .expect("prepare second recent network topic connection");
        let second_lease = hub
            .register_server_push_topics(state, vec![topic])
            .await
            .expect("register second recent network push topic");

        {
            let guard = hub.state.lock().await;
            assert_eq!(
                guard.server_push_subscribers.get(&topic_key).copied(),
                Some(2)
            );
            assert_eq!(guard.server_push_tasks.len(), 1);
            assert!(guard.server_push_tasks.contains(&topic_key));
        }

        drop(first_lease);
        tokio::time::sleep(Duration::from_millis(10)).await;
        {
            let guard = hub.state.lock().await;
            assert_eq!(
                guard.server_push_subscribers.get(&topic_key).copied(),
                Some(1)
            );
            assert_eq!(guard.server_push_tasks.len(), 1);
        }

        drop(second_lease);
        tokio::time::sleep(DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL * 2).await;
        let guard = hub.state.lock().await;
        assert_eq!(guard.server_push_subscribers.get(&topic_key), None);
        assert!(!guard.server_push_tasks.contains(&topic_key));
    }

    #[test]
    fn prune_replay_window_enforces_event_cap() {
        let mut events = VecDeque::new();
        let mut total_bytes = 0usize;
        for index in 0..(SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC + 8) {
            events.push_back(ReplayableTopicEvent {
                cursor: index as u64 + 1,
                payload: json!({ "cursor": index + 1 }),
                bytes: 32,
                emitted_at: Utc::now(),
            });
            total_bytes += 32;
        }

        prune_replay_window(&mut events, &mut total_bytes);

        assert!(events.len() <= SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC);
    }

    #[test]
    fn prune_replay_window_drops_expired_entries() {
        let now = Utc::now();
        let mut events = VecDeque::from([
            ReplayableTopicEvent {
                cursor: 1,
                payload: json!({ "cursor": 1 }),
                bytes: 32,
                emitted_at: now - ChronoDuration::seconds(SUBSCRIPTION_REPLAY_WINDOW_SECS + 5),
            },
            ReplayableTopicEvent {
                cursor: 2,
                payload: json!({ "cursor": 2 }),
                bytes: 32,
                emitted_at: now,
            },
        ]);
        let mut total_bytes = 64usize;

        prune_replay_window(&mut events, &mut total_bytes);

        assert_eq!(events.len(), 1);
        assert_eq!(events.front().map(|event| event.cursor), Some(2));
        assert_eq!(total_bytes, 32);
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
            replay.iter().map(|event| event.cursor).collect::<Vec<_>>(),
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
