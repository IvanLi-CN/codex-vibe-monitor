impl DashboardTopicMaterializer {
    fn revision(
        &self,
        base_revision: u64,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Option<DashboardTopicRevision> {
        match self {
            Self::Activity { base, .. } => {
                let routing_revision = base
                    .lock()
                    .expect("activity materializer state lock")
                    .routing_revision;
                if current.is_none()
                    && network.is_none()
                    && terminal.is_none()
                    && routing_revision == 0
                {
                    return None;
                }
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: network.map(|slice| slice.revision),
                    terminal_revision: terminal.map(|slice| slice.revision),
                    routing_revision,
                })
            }
            Self::Summary { .. } if current.is_some() || terminal.is_some() => {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: None,
                    terminal_revision: terminal.map(|slice| slice.revision),
                    routing_revision: 0,
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
                    routing_revision: 0,
                }),
            Self::NetworkRecent { .. } => network.map(|slice| DashboardTopicRevision {
                base_revision,
                current_revision: None,
                network_revision: Some(slice.revision),
                terminal_revision: None,
                routing_revision: 0,
            }),
            Self::Timeseries { .. } if current.is_some() || terminal.is_some() => {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: None,
                    terminal_revision: terminal.map(|slice| slice.revision),
                    routing_revision: 0,
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
                routing_revision: 0,
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
                window: _,
                reporting_tz: _,
                source_scope: _,
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
                    // Terminal slices can be emitted before the SQLite transaction commits.
                    // Summary only incorporates the writer-ACKed journal during its dedicated
                    // refresh, so this delivery path never publishes a speculative total.
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
    UpstreamAccountAttemptsWindow {
        account_id: i64,
        page: usize,
        page_size: usize,
        attempt_type: Option<String>,
        model: Option<String>,
        sticky_key: Option<String>,
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
