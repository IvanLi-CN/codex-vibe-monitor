use super::*;

#[derive(Debug, Clone)]

pub(crate) struct PendingDashboardTopicMaterialization {
    pub(crate) topic_key: String,
    pub(crate) topic_name: &'static str,
    pub(crate) revision: DashboardTopicRevision,
    pub(crate) materializer: DashboardTopicMaterializer,
}

pub(crate) struct ServerPushTopicLease {
    pub(crate) hub: Arc<SubscriptionHub>,
    pub(crate) topic_keys: Vec<String>,
}

pub(crate) struct TopicSubscriptionLease {
    pub(crate) hub: Arc<SubscriptionHub>,
    pub(crate) topic_keys: Vec<String>,
    pub(crate) topic_names: Vec<String>,
    pub(crate) owns_dashboard_live: bool,
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
    pub(crate) fn as_str(&self) -> &'static str {
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
    Unavailable,
}

impl TopicInitDisposition {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::ReplayHit => "replay_hit",
            Self::ResumeCaughtUp => "resume_caught_up",
            Self::SnapshotNoResume => "snapshot_no_resume",
            Self::SnapshotResumeMiss => "snapshot_resume_miss",
            Self::Unavailable => "unavailable",
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
    pub(crate) kind: TopicFrameKind,
}

#[derive(Debug, Clone)]
pub(crate) enum SubscriptionTopic {
    AppVersion,
    QuotaCurrent,
    ManagedTaskCatalog,
    ManagedTaskRuntime,
    ManagedTaskTimeline,
    ManagedTaskTimelineV2,
    ManagedTaskDetail {
        task_key: String,
    },
    ManagedTaskWorkload {
        task_key: String,
        window_hours: i64,
        limit: usize,
    },
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
pub(crate) enum ConversationSubscriptionScope {
    PromptCacheKey(String),
    StickyKey {
        sticky_key: String,
        upstream_account_id: i64,
    },
}

impl ConversationSubscriptionScope {
    pub(crate) fn binding_key(&self) -> &str {
        match self {
            Self::PromptCacheKey(prompt_cache_key) => prompt_cache_key,
            Self::StickyKey { sticky_key, .. } => sticky_key,
        }
    }

    pub(crate) fn matches_record(&self, record: &ApiInvocation) -> bool {
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

    pub(crate) fn matches_runtime_mutation(&self, mutation: &RuntimeInvocationMutation) -> bool {
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

    pub(crate) fn matches_sticky_route_change(
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

    pub(crate) fn descriptor_params(&self) -> BTreeMap<String, String> {
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

    pub(crate) fn list_query(
        &self,
        page: i64,
        page_size: i64,
        snapshot_id: Option<i64>,
    ) -> ListQuery {
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

impl SubscriptionTopic {
    pub(crate) fn class(&self) -> SubscriptionTopicClass {
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
            | Self::ManagedTaskCatalog
            | Self::ManagedTaskRuntime
            | Self::ManagedTaskTimeline
            | Self::ManagedTaskTimelineV2
            | Self::ManagedTaskDetail { .. }
            | Self::ManagedTaskWorkload { .. }
            | Self::InvocationWindow { .. }
            | Self::InvocationHistoryWindow { .. }
            | Self::InvocationHistoryOverview { .. }
            | Self::PromptCacheConversationBindingCurrent { .. }
            | Self::PromptCacheConversationOperationsWindow { .. }
            | Self::PromptCacheWindow { .. }
            | Self::PromptCacheStickyWindow { .. }
            | Self::ForwardProxyLive
            | Self::InvocationPoolAttempts { .. }
            | Self::ModelRoutingLive { .. }
            | Self::UpstreamAccountAttemptsWindow { .. } => {
                SubscriptionTopicClass::BoundedColdHydrate
            }
        }
    }

    pub(crate) fn uses_server_push_cadence(&self, mode: RuntimeProjectionMode) -> bool {
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

    pub(crate) fn uses_dashboard_activity_live_overlay(&self) -> bool {
        matches!(
            self,
            Self::DashboardActivityCurrent { range, .. } if range != "yesterday"
        )
    }

    pub(crate) fn uses_summary_live_overlay(&self) -> bool {
        matches!(
            self,
            Self::SummaryCurrent { window, .. }
                if !matches!(window.as_str(), "yesterday" | "previous7d")
        )
    }

    pub(crate) fn uses_timeseries_live_projection(&self) -> bool {
        matches!(self, Self::TimeseriesOpenWindow { range, .. } if range != "yesterday")
    }

    pub(crate) fn uses_summary_topic_refresh(&self) -> bool {
        self.uses_summary_live_overlay()
    }

    pub(crate) fn uses_conversation_overview_refresh(&self) -> bool {
        matches!(self, Self::InvocationHistoryOverview { .. })
    }

    pub(crate) fn uses_upstream_account_attempt_refresh(&self) -> bool {
        matches!(self, Self::UpstreamAccountAttemptsWindow { .. })
    }

    pub(crate) fn is_unmigrated_dashboard_hot_projection(&self) -> bool {
        false
    }

    pub(crate) fn uses_parallel_work_live_projection(&self) -> bool {
        matches!(self, Self::ParallelWorkCurrent { range, .. } if range != "yesterday")
    }

    pub(crate) fn is_closed_dashboard_hot_snapshot(&self) -> bool {
        matches!(
            self,
            Self::ParallelWorkCurrent { range, .. } | Self::TimeseriesOpenWindow { range, .. }
                if range == "yesterday"
        )
    }

    pub(crate) fn uses_dashboard_network_live_snapshot(&self) -> bool {
        matches!(
            self,
            Self::DashboardNetworkTimeseriesWindow { .. } | Self::DashboardNetworkRecentCurrent
        )
    }

    pub(crate) fn runtime_topic_dependencies(&self) -> Vec<RuntimeTopicDependency> {
        match self {
            // These topics receive their revisions from the RuntimeProjectionHub directly, so
            // they intentionally do not create generic router work.
            Self::DashboardActivityCurrent { .. }
            | Self::DashboardNetworkTimeseriesWindow { .. }
            | Self::DashboardNetworkRecentCurrent
            | Self::SummaryCurrent { .. }
            | Self::AppVersion
            | Self::QuotaCurrent
            | Self::ManagedTaskCatalog
            | Self::ManagedTaskRuntime
            | Self::ManagedTaskTimeline
            | Self::ManagedTaskTimelineV2
            | Self::ManagedTaskDetail { .. }
            | Self::ManagedTaskWorkload { .. } => Vec::new(),
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
            Self::UpstreamAccountAttemptsWindow { .. } => Vec::new(),
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

    pub(crate) fn from_descriptor(
        descriptor: &SubscriptionTopicDescriptor,
    ) -> Result<Self, ApiError> {
        let topic = descriptor.topic.trim();
        let params = &descriptor.params;
        match topic {
            "app.version" => Ok(Self::AppVersion),
            "quota.current" => Ok(Self::QuotaCurrent),
            "system.managed-tasks.catalog" => Ok(Self::ManagedTaskCatalog),
            "system.managed-tasks.runtime" => Ok(Self::ManagedTaskRuntime),
            "system.managed-tasks.timeline" => {
                match parse_i64_param(params, "schemaVersion", Some(1))? {
                    1 => Ok(Self::ManagedTaskTimeline),
                    2 => Ok(Self::ManagedTaskTimelineV2),
                    _ => Err(ApiError::bad_request(anyhow!(
                        "managed task timeline schemaVersion must be 1 or 2"
                    ))),
                }
            }
            "system.managed-tasks.detail" => Ok(Self::ManagedTaskDetail {
                task_key: parse_required_text_param(params, "taskKey")?,
            }),
            "system.managed-tasks.workload" => {
                let window_hours = parse_i64_param(params, "windowHours", Some(24))?;
                if window_hours != 24 {
                    return Err(ApiError::bad_request(anyhow!(
                        "workload windowHours must be exactly 24"
                    )));
                }
                let limit = parse_i64_param(params, "limit", Some(200))?;
                if !(1..=200).contains(&limit) {
                    return Err(ApiError::bad_request(anyhow!(
                        "workload limit must be between 1 and 200"
                    )));
                }
                Ok(Self::ManagedTaskWorkload {
                    task_key: parse_required_text_param(params, "taskKey")?,
                    window_hours,
                    limit: limit as usize,
                })
            }
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
            "upstream-account-attempts.window" => {
                let account_id = parse_required_positive_i64_param(params, "accountId")?;
                let page = parse_upstream_account_attempt_page_param(params, "page", 1)?;
                let page_size = parse_upstream_account_attempt_page_param(params, "pageSize", 20)?;
                Ok(Self::UpstreamAccountAttemptsWindow {
                    account_id,
                    page,
                    page_size,
                    attempt_type: normalize_upstream_account_attempt_type_param(params),
                    model: parse_optional_text_param(params, "model"),
                    sticky_key: parse_optional_text_param(params, "stickyKey"),
                })
            }
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported subscription topic: {topic}"
            ))),
        }
    }

    pub(crate) fn descriptor(&self) -> SubscriptionTopicDescriptor {
        match self {
            Self::AppVersion => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::QuotaCurrent => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::ManagedTaskCatalog | Self::ManagedTaskRuntime | Self::ManagedTaskTimeline => {
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params: BTreeMap::new(),
                }
            }
            Self::ManagedTaskTimelineV2 => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: btree_map_from_pairs([("schemaVersion", "2".to_string())]),
            },
            Self::ManagedTaskDetail { task_key } => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: btree_map_from_pairs([("taskKey", task_key.clone())]),
            },
            Self::ManagedTaskWorkload {
                task_key,
                window_hours,
                limit,
            } => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: btree_map_from_pairs([
                    ("taskKey", task_key.clone()),
                    ("windowHours", window_hours.to_string()),
                    ("limit", limit.to_string()),
                ]),
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
            Self::UpstreamAccountAttemptsWindow {
                account_id,
                page,
                page_size,
                attempt_type,
                model,
                sticky_key,
            } => {
                let mut params = btree_map_from_pairs([
                    ("accountId", account_id.to_string()),
                    ("page", page.to_string()),
                    ("pageSize", page_size.to_string()),
                ]);
                insert_optional_param(&mut params, "type", attempt_type.clone());
                insert_optional_param(&mut params, "model", model.clone());
                insert_optional_param(&mut params, "stickyKey", sticky_key.clone());
                SubscriptionTopicDescriptor {
                    topic: self.name().to_string(),
                    params,
                }
            }
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::AppVersion => "app.version",
            Self::QuotaCurrent => "quota.current",
            Self::ManagedTaskCatalog => "system.managed-tasks.catalog",
            Self::ManagedTaskRuntime => "system.managed-tasks.runtime",
            Self::ManagedTaskTimeline => "system.managed-tasks.timeline",
            Self::ManagedTaskTimelineV2 => "system.managed-tasks.timeline",
            Self::ManagedTaskDetail { .. } => "system.managed-tasks.detail",
            Self::ManagedTaskWorkload { .. } => "system.managed-tasks.workload",
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
            Self::UpstreamAccountAttemptsWindow { .. } => "upstream-account-attempts.window",
        }
    }

    pub(crate) fn schema_epoch(&self) -> String {
        match self {
            Self::AppVersion => "app.version/v1".to_string(),
            Self::QuotaCurrent => "quota.current/v1".to_string(),
            Self::ManagedTaskCatalog => "system.managed-tasks.catalog/v1".to_string(),
            Self::ManagedTaskRuntime => "system.managed-tasks.runtime/v1".to_string(),
            Self::ManagedTaskTimeline => "system.managed-tasks.timeline/v1".to_string(),
            Self::ManagedTaskTimelineV2 => "system.managed-tasks.timeline/v2".to_string(),
            Self::ManagedTaskDetail { .. } => "system.managed-tasks.detail/v1".to_string(),
            Self::ManagedTaskWorkload { .. } => "system.managed-tasks.workload/v1".to_string(),
            Self::DashboardActivityCurrent { .. } => "dashboard.activity.current/v3".to_string(),
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
            Self::UpstreamAccountAttemptsWindow { .. } => {
                "upstream-account-attempts.window/v1".to_string()
            }
        }
    }

    pub(crate) fn cache_key(&self) -> Result<String, ApiError> {
        serde_json::to_string(&self.descriptor()).map_err(ApiError::from)
    }

    pub(crate) fn is_closed_summary_topic(&self) -> bool {
        matches!(
            self,
            Self::SummaryCurrent { window, .. }
                if matches!(window.as_str(), "yesterday" | "previous7d")
        )
    }

    pub(crate) fn is_affected_by_runtime_mutation(&self, mutation: &RuntimeMutation) -> bool {
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
                    | Self::ManagedTaskCatalog
                    | Self::ManagedTaskRuntime
                    | Self::ManagedTaskTimeline
                    | Self::ManagedTaskTimelineV2
                    | Self::ManagedTaskDetail { .. }
                    | Self::ManagedTaskWorkload { .. }
                    | Self::PromptCacheConversationBindingCurrent { .. }
                    | Self::PromptCacheConversationOperationsWindow { .. }
                    | Self::PromptCacheWindow { .. }
                    | Self::PromptCacheStickyWindow { .. }
                    | Self::InvocationPoolAttempts { .. }
                    | Self::ModelRoutingLive { .. }
                    | Self::UpstreamAccountAttemptsWindow { .. } => false,
                }
            }
            RuntimeMutation::AttemptChanged { invoke_id } => matches!(
                self,
                Self::InvocationPoolAttempts { invoke_id: current } if current == invoke_id
            ),
            RuntimeMutation::ModelRoutingChanged => matches!(self, Self::ModelRoutingLive { .. }),
            RuntimeMutation::AccountEffectiveRoutingRulesChanged { .. } => false,
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

    pub(crate) fn is_affected_by(&self, payload: &BroadcastPayload) -> bool {
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
            | BroadcastPayload::PromptCacheConversationChanged { .. }
            | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. } => false,
            BroadcastPayload::PoolAttemptsSnapshotUnavailable { .. } => false,
            BroadcastPayload::PoolAttempts { attempts, .. } => match self {
                Self::UpstreamAccountAttemptsWindow { account_id, .. } => attempts
                    .iter()
                    .any(|attempt| attempt.upstream_account_id == Some(*account_id)),
                _ => false,
            },
        }
    }
}

pub(crate) async fn build_managed_task_timeline_topic_payload(
    after_revision: Option<i64>,
) -> Result<Value, ApiError> {
    let Some(store) = crate::maintenance_store::global() else {
        return Err(ApiError::unavailable(anyhow!(
            "maintenance database unavailable"
        )));
    };
    managed_task_timeline_page_payload(store, after_revision).await
}

pub(crate) async fn managed_task_timeline_page_payload(
    store: &crate::maintenance_store::MaintenanceStore,
    after_revision: Option<i64>,
) -> Result<Value, ApiError> {
    let window_end = Utc::now();
    let window_start = window_end - ChronoDuration::hours(12);
    let from = format_utc_iso_millis(window_start);
    let to = format_utc_iso_millis(window_end);
    let mut page = crate::task_timeline::timeline_page(
        store,
        None,
        after_revision,
        Some(&from),
        Some(&to),
        None,
        500,
    )
    .await
    .map_err(ApiError::from)?;
    let mut segments = std::mem::take(&mut page.segments);
    let mut cursor = page.next_cursor.take();
    while let Some(next_cursor) = cursor {
        let next = crate::task_timeline::timeline_page(
            store,
            Some(&next_cursor),
            None,
            None,
            None,
            None,
            500,
        )
        .await
        .map_err(ApiError::from)?;
        if next.reset_required {
            return Err(ApiError::unavailable(anyhow!(
                "managed task timeline pagination expired; a fresh snapshot is required"
            )));
        }
        segments.extend(next.segments);
        cursor = next.next_cursor;
        if segments.len() > MANAGED_TASK_TIMELINE_TOPIC_SEGMENT_LIMIT
            || (segments.len() == MANAGED_TASK_TIMELINE_TOPIC_SEGMENT_LIMIT && cursor.is_some())
        {
            return Err(ApiError::unavailable(anyhow!(
                "managed task timeline exceeds the bounded SSE snapshot capacity"
            )));
        }
    }
    page.segments = segments;
    page.next_cursor = None;
    let mut payload = serde_json::to_value(page)?;
    if let Some(object) = payload.as_object_mut() {
        object.insert("replace".to_string(), Value::Bool(after_revision.is_none()));
    }
    Ok(payload)
}

pub(crate) async fn build_managed_task_timeline_revision_topic_payload() -> Result<Value, ApiError>
{
    let Some(store) = crate::maintenance_store::global() else {
        return Err(ApiError::unavailable(anyhow!(
            "maintenance database unavailable"
        )));
    };
    managed_task_timeline_revision_payload(store).await
}

pub(crate) async fn managed_task_timeline_revision_payload(
    store: &crate::maintenance_store::MaintenanceStore,
) -> Result<Value, ApiError> {
    let watermark = store.timeline_revision().await.map_err(ApiError::from)?;
    Ok(serde_json::json!({
        "watermark": watermark,
        "observedAt": format_utc_iso_millis(Utc::now()),
    }))
}

pub(crate) fn merge_managed_task_timeline_payload(
    previous: &Value,
    update: &Value,
) -> Result<Value, ApiError> {
    if update.get("replace").and_then(Value::as_bool) != Some(false) {
        return Ok(update.clone());
    }
    let previous_segments = previous
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ApiError::unavailable(anyhow!("cached task timeline snapshot is invalid"))
        })?;
    let update_segments = update
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::unavailable(anyhow!("task timeline delta is invalid")))?;
    let window_start = update
        .get("windowStart")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let window_end = update
        .get("windowEnd")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc));
    let mut by_id = HashMap::<String, Value>::new();
    for segment in previous_segments.iter().chain(update_segments) {
        let Some(id) = segment.get("segmentId").and_then(Value::as_str) else {
            return Err(ApiError::unavailable(anyhow!(
                "task timeline segment identity is missing"
            )));
        };
        by_id.insert(id.to_string(), segment.clone());
    }
    let mut segments = by_id
        .into_values()
        .filter(|segment| {
            let started_at = segment
                .get("startedAt")
                .and_then(Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc));
            let ended_at = segment
                .get("finishedAt")
                .and_then(Value::as_str)
                .or_else(|| segment.get("lastObservedAt").and_then(Value::as_str))
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc));
            match (window_start, window_end, started_at, ended_at) {
                (Some(start), Some(end), Some(segment_start), Some(segment_end)) => {
                    segment_start <= end && segment_end >= start
                }
                _ => true,
            }
        })
        .collect::<Vec<_>>();
    segments.sort_by(|left, right| {
        left.get("startedAt")
            .and_then(Value::as_str)
            .cmp(&right.get("startedAt").and_then(Value::as_str))
    });
    let mut merged = update.clone();
    let Some(object) = merged.as_object_mut() else {
        return Err(ApiError::unavailable(anyhow!(
            "task timeline delta is invalid"
        )));
    };
    object.insert("segments".to_string(), Value::Array(segments));
    object.insert("replace".to_string(), Value::Bool(true));
    Ok(merged)
}

impl RuntimeMutation {
    pub(crate) fn topic_dependencies(&self) -> Vec<RuntimeTopicDependency> {
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
            Self::AccountEffectiveRoutingRulesChanged { .. } => Vec::new(),
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
