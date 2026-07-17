use super::*;
use serde::de::DeserializeOwned;

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
}

#[derive(Debug, Clone)]
struct CachedSubscriptionTopic {
    topic: SubscriptionTopic,
    descriptor: SubscriptionTopicDescriptor,
    schema_epoch: String,
    cursor: u64,
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

#[derive(Debug, Clone)]
pub(crate) enum ReplayMissReason {
    SchemaEpochMismatch,
    GapWindowMiss,
    GapEventBudgetExceeded,
    GapByteBudgetExceeded,
    UnknownTopic,
}

impl ReplayMissReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::SchemaEpochMismatch => "schema_epoch_mismatch",
            Self::GapWindowMiss => "gap_window_miss",
            Self::GapEventBudgetExceeded => "gap_event_budget_exceeded",
            Self::GapByteBudgetExceeded => "gap_byte_budget_exceeded",
            Self::UnknownTopic => "unknown_topic",
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
    DashboardWorkingConversationsCurrent {
        page_size: i64,
        recent_invocation_limit: i64,
    },
    InvocationWindow {
        limit: i64,
        model: Option<String>,
        status: Option<String>,
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
            let replay_attempt = self
                .replay_events_for_resume(&topic_key, topic.schema_epoch(), resume_cursor)
                .await;

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
        if let Some(existing) = self.state.lock().await.topics.get(&topic_key).cloned() {
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
        let topic_key = topic.cache_key()?;
        let schema_epoch = topic.schema_epoch();
        let descriptor = topic.descriptor();
        let started = Instant::now();
        let payload = topic.build_payload(state.clone()).await?;
        let payload_bytes = serialized_len(&payload)?;

        let (cached, dispatch) = {
            let mut guard = self.state.lock().await;
            let current_cursor = guard.topics.get(&topic_key).map_or(0, |entry| entry.cursor);
            let next_cursor = current_cursor.saturating_add(1);
            let mut next = CachedSubscriptionTopic {
                topic: topic.clone(),
                descriptor: descriptor.clone(),
                schema_epoch: schema_epoch.clone(),
                cursor: next_cursor,
                snapshot_payload: payload.clone(),
                snapshot_bytes: payload_bytes,
                replay_events: guard
                    .topics
                    .get(&topic_key)
                    .map(|entry| entry.replay_events.clone())
                    .unwrap_or_default(),
                replay_bytes: guard
                    .topics
                    .get(&topic_key)
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

        Ok(cached)
    }

    pub(crate) async fn handle_internal_broadcast(
        &self,
        state: Arc<AppState>,
        payload: BroadcastPayload,
    ) {
        let affected = {
            let guard = self.state.lock().await;
            guard
                .topics
                .values()
                .filter(|cached| cached.topic.is_affected_by(&payload))
                .map(|cached| cached.topic.clone())
                .collect::<Vec<_>>()
        };

        for topic in affected {
            if let Err(err) = self.refresh_topic(state.clone(), topic.clone(), true).await {
                warn!(
                    ?err,
                    topic = %topic.name(),
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
    let resume = decode_resume_query(query.resume.as_deref())?;
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
    let prepared = state
        .subscription_hub
        .prepare_connection(state.clone(), descriptors, resume)
        .await?;
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

    let initial_stream = stream::iter(
        initial
            .into_iter()
            .filter_map(|payload| serialize_sse_event(&payload).ok()),
    );

    let live_stream = async_stream::stream! {
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
            Self::DashboardWorkingConversationsCurrent {
                page_size,
                recent_invocation_limit,
            } => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: btree_map_from_pairs([
                    ("pageSize", page_size.to_string()),
                    ("recentInvocationLimit", recent_invocation_limit.to_string()),
                ]),
            },
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
            Self::DashboardWorkingConversationsCurrent { .. } => {
                "dashboard.working-conversations.current"
            }
            Self::InvocationWindow { .. } => "invocations.window",
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
            Self::DashboardWorkingConversationsCurrent { .. } => {
                "dashboard.working-conversations.current/v1".to_string()
            }
            Self::InvocationWindow { .. } => "invocations.window/v1".to_string(),
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

    fn is_affected_by(&self, payload: &BroadcastPayload) -> bool {
        match payload {
            BroadcastPayload::Records { .. } => matches!(
                self,
                Self::DashboardActivityCurrent { .. }
                    | Self::DashboardNetworkTimeseriesWindow { .. }
                    | Self::DashboardWorkingConversationsCurrent { .. }
                    | Self::InvocationWindow { .. }
                    | Self::PromptCacheWindow { .. }
                    | Self::PromptCacheStickyWindow { .. }
                    | Self::SummaryCurrent { .. }
                    | Self::TimeseriesOpenWindow { .. }
                    | Self::ParallelWorkCurrent { .. }
                    | Self::ForwardProxyLive
            ),
            BroadcastPayload::DashboardActivityLive { .. } => {
                matches!(
                    self,
                    Self::DashboardActivityCurrent { .. }
                        | Self::DashboardNetworkTimeseriesWindow { .. }
                )
            }
            BroadcastPayload::PoolAttempts { invoke_id, .. } => matches!(
                self,
                Self::InvocationPoolAttempts { invoke_id: current } if current == invoke_id
            ),
            BroadcastPayload::Summary { .. } => matches!(self, Self::SummaryCurrent { .. }),
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
            Self::DashboardWorkingConversationsCurrent {
                page_size,
                recent_invocation_limit,
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
                        requester_ip: None,
                        keyword: None,
                        min_total_tokens: None,
                        max_total_tokens: None,
                        min_total_ms: None,
                        max_total_ms: None,
                        suggest_field: None,
                        suggest_query: None,
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
                let Json(response) = fetch_summary(
                    State(state),
                    Query(SummaryQuery {
                        window: Some(window.clone()),
                        limit: *limit,
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    }),
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

fn decode_resume_query(raw: Option<&str>) -> Result<Vec<SubscriptionResumeCursor>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    decode_query_json(raw, "resume")
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
}
