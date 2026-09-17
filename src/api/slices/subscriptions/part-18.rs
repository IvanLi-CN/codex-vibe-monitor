fn descriptor_dashboard_activity(
    topic: &SubscriptionTopic,
    range: &str,
    time_zone: &str,
    recent_limit: i64,
    include_accounts: bool,
    include_recent: bool,
) -> SubscriptionTopicDescriptor {
    SubscriptionTopicDescriptor {
        topic: topic.name().to_string(),
        params: btree_map_from_pairs([
            ("range", range.to_string()),
            ("timeZone", time_zone.to_string()),
            ("recentLimit", recent_limit.to_string()),
            ("includeAccounts", include_accounts.to_string()),
            ("includeRecent", include_recent.to_string()),
        ]),
    }
}

fn descriptor_dashboard_network_timeseries(
    topic: &SubscriptionTopic,
    range: &str,
    time_zone: &str,
    upstream_account_id: Option<i64>,
) -> SubscriptionTopicDescriptor {
    let mut params = btree_map_from_pairs([
        ("range", range.to_string()),
        ("timeZone", time_zone.to_string()),
    ]);
    insert_optional_param(
        &mut params,
        "upstreamAccountId",
        upstream_account_id.map(|value| value.to_string()),
    );
    SubscriptionTopicDescriptor {
        topic: topic.name().to_string(),
        params,
    }
}

fn descriptor_dashboard_working_conversations(
    topic: &SubscriptionTopic,
    page_size: i64,
    recent_invocation_limit: i64,
    blocked_binding_upstream_account_id: Option<i64>,
    blocked_binding_constraint_source: Option<BlockedBindingConstraintSource>,
) -> SubscriptionTopicDescriptor {
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
        topic: topic.name().to_string(),
        params,
    }
}

fn descriptor_scope_topic(
    topic: &SubscriptionTopic,
    params: BTreeMap<String, String>,
) -> SubscriptionTopicDescriptor {
    SubscriptionTopicDescriptor {
        topic: topic.name().to_string(),
        params,
    }
}

fn descriptor_invocation_window(
    topic: &SubscriptionTopic,
    limit: i64,
    model: &Option<String>,
    status: &Option<String>,
) -> SubscriptionTopicDescriptor {
    let mut params = btree_map_from_pairs([("limit", limit.to_string())]);
    insert_optional_param(&mut params, "model", model.clone());
    insert_optional_param(&mut params, "status", status.clone());
    descriptor_scope_topic(topic, params)
}

fn descriptor_scope_with_info(
    topic: &SubscriptionTopic,
    scope: &ConversationSubscriptionScope,
    info_type: &Option<String>,
) -> SubscriptionTopicDescriptor {
    let mut params = scope.descriptor_params();
    insert_optional_param(&mut params, "infoType", info_type.clone());
    descriptor_scope_topic(topic, params)
}

fn descriptor_prompt_cache_window(
    topic: &SubscriptionTopic,
    selection: PromptCacheConversationSelection,
    detail_level: &PromptCacheConversationDetailLevel,
    recent_invocation_limit: Option<i64>,
) -> SubscriptionTopicDescriptor {
    let mut params = prompt_cache_selection_params(selection);
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
    descriptor_scope_topic(topic, params)
}

fn descriptor_prompt_cache_sticky(
    topic: &SubscriptionTopic,
    account_id: i64,
    selection: AccountStickyKeySelection,
) -> SubscriptionTopicDescriptor {
    let mut params = BTreeMap::from([("accountId".to_string(), account_id.to_string())]);
    match selection {
        AccountStickyKeySelection::Count(limit) => {
            params.insert("limit".to_string(), limit.to_string());
        }
        AccountStickyKeySelection::ActivityWindow(hours) => {
            params.insert("activityHours".to_string(), hours.to_string());
        }
    }
    descriptor_scope_topic(topic, params)
}

fn descriptor_summary(
    topic: &SubscriptionTopic,
    window: &str,
    time_zone: &str,
    limit: Option<i64>,
    upstream_account_id: Option<i64>,
) -> SubscriptionTopicDescriptor {
    let mut params = btree_map_from_pairs([
        ("window", window.to_string()),
        ("timeZone", time_zone.to_string()),
    ]);
    insert_optional_param(&mut params, "limit", limit.map(|value| value.to_string()));
    insert_optional_param(
        &mut params,
        "upstreamAccountId",
        upstream_account_id.map(|value| value.to_string()),
    );
    descriptor_scope_topic(topic, params)
}

fn descriptor_timeseries(
    topic: &SubscriptionTopic,
    range: &str,
    time_zone: &str,
    bucket: &Option<String>,
    settlement_hour: Option<u8>,
    upstream_account_id: Option<i64>,
) -> SubscriptionTopicDescriptor {
    let mut params = btree_map_from_pairs([
        ("range", range.to_string()),
        ("timeZone", time_zone.to_string()),
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
    descriptor_scope_topic(topic, params)
}

fn descriptor_parallel_work(
    topic: &SubscriptionTopic,
    range: &str,
    time_zone: &str,
    bucket: &Option<String>,
    upstream_account_id: Option<i64>,
) -> SubscriptionTopicDescriptor {
    let mut params = btree_map_from_pairs([
        ("range", range.to_string()),
        ("timeZone", time_zone.to_string()),
    ]);
    insert_optional_param(&mut params, "bucket", bucket.clone());
    insert_optional_param(
        &mut params,
        "upstreamAccountId",
        upstream_account_id.map(|value| value.to_string()),
    );
    descriptor_scope_topic(topic, params)
}

fn descriptor_model_routing(
    topic: &SubscriptionTopic,
    window: &str,
    model: &Option<String>,
    state: &Option<String>,
    limit: i64,
) -> SubscriptionTopicDescriptor {
    let mut params =
        btree_map_from_pairs([("window", window.to_string()), ("limit", limit.to_string())]);
    insert_optional_param(&mut params, "model", model.clone());
    insert_optional_param(&mut params, "state", state.clone());
    descriptor_scope_topic(topic, params)
}

fn descriptor_upstream_attempts(
    topic: &SubscriptionTopic,
    account_id: i64,
    page: usize,
    page_size: usize,
    attempt_type: &Option<String>,
    model: &Option<String>,
    sticky_key: &Option<String>,
) -> SubscriptionTopicDescriptor {
    let mut params = btree_map_from_pairs([
        ("accountId", account_id.to_string()),
        ("page", page.to_string()),
        ("pageSize", page_size.to_string()),
    ]);
    insert_optional_param(&mut params, "type", attempt_type.clone());
    insert_optional_param(&mut params, "model", model.clone());
    insert_optional_param(&mut params, "stickyKey", sticky_key.clone());
    descriptor_scope_topic(topic, params)
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
            | Self::ModelRoutingLive { .. }
            | Self::UpstreamAccountAttemptsWindow { .. } => {
                SubscriptionTopicClass::BoundedColdHydrate
            }
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

    fn uses_upstream_account_attempt_refresh(&self) -> bool {
        matches!(self, Self::UpstreamAccountAttemptsWindow { .. })
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
                parse_dashboard_working_conversations_topic(params)
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
            "pool.model-routing-live" => parse_model_routing_live_topic(params),
            "upstream-account-attempts.window" => parse_upstream_account_attempts_topic(params),
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported subscription topic: {topic}"
            ))),
        }
    }
}

fn parse_dashboard_working_conversations_topic(
    params: &BTreeMap<String, String>,
) -> Result<SubscriptionTopic, ApiError> {
    let page_size = normalize_prompt_cache_conversation_page_size(Some(parse_i64_param(
        params,
        "pageSize",
        Some(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE),
    )?))?
    .unwrap_or(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE);
    let recent_invocation_limit =
        normalize_prompt_cache_conversation_recent_invocation_limit(Some(parse_i64_param(
            params,
            "recentInvocationLimit",
            Some(SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT),
        )?))?
        .unwrap_or(SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT);
    Ok(SubscriptionTopic::DashboardWorkingConversationsCurrent {
        page_size,
        recent_invocation_limit,
        blocked_binding_upstream_account_id: parse_optional_i64_param(
            params,
            "blockedBindingUpstreamAccountId",
        )?,
        blocked_binding_constraint_source: parse_optional_blocked_binding_constraint_source_param(
            params,
            "blockedBindingConstraintSource",
        )?,
    })
}

fn parse_model_routing_live_topic(
    params: &BTreeMap<String, String>,
) -> Result<SubscriptionTopic, ApiError> {
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
    Ok(SubscriptionTopic::ModelRoutingLive {
        window,
        model: parse_optional_text_param(params, "model"),
        state,
        limit,
    })
}

fn parse_upstream_account_attempts_topic(
    params: &BTreeMap<String, String>,
) -> Result<SubscriptionTopic, ApiError> {
    let account_id = parse_required_positive_i64_param(params, "accountId")?;
    Ok(SubscriptionTopic::UpstreamAccountAttemptsWindow {
        account_id,
        page: parse_upstream_account_attempt_page_param(params, "page", 1)?,
        page_size: parse_upstream_account_attempt_page_param(params, "pageSize", 20)?,
        attempt_type: normalize_upstream_account_attempt_type_param(params),
        model: parse_optional_text_param(params, "model"),
        sticky_key: parse_optional_text_param(params, "stickyKey"),
    })
}

impl SubscriptionTopic {
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
            } => descriptor_dashboard_activity(
                self,
                range,
                time_zone,
                *recent_limit,
                *include_accounts,
                *include_recent,
            ),
            Self::DashboardNetworkTimeseriesWindow {
                range,
                time_zone,
                upstream_account_id,
            } => descriptor_dashboard_network_timeseries(
                self,
                range,
                time_zone,
                *upstream_account_id,
            ),
            Self::DashboardNetworkRecentCurrent => SubscriptionTopicDescriptor {
                topic: self.name().to_string(),
                params: BTreeMap::new(),
            },
            Self::DashboardWorkingConversationsCurrent {
                page_size,
                recent_invocation_limit,
                blocked_binding_upstream_account_id,
                blocked_binding_constraint_source,
            } => descriptor_dashboard_working_conversations(
                self,
                *page_size,
                *recent_invocation_limit,
                *blocked_binding_upstream_account_id,
                *blocked_binding_constraint_source,
            ),
            Self::InvocationWindow {
                limit,
                model,
                status,
            } => descriptor_invocation_window(self, *limit, model, status),
            Self::InvocationHistoryWindow { .. }
            | Self::InvocationHistoryOverview { .. }
            | Self::PromptCacheConversationBindingCurrent { .. }
            | Self::PromptCacheConversationOperationsWindow { .. }
            | Self::PromptCacheWindow { .. }
            | Self::PromptCacheStickyWindow { .. } => descriptor_history_and_cache(self),
            Self::SummaryCurrent { .. }
            | Self::TimeseriesOpenWindow { .. }
            | Self::ParallelWorkCurrent { .. }
            | Self::ForwardProxyLive
            | Self::InvocationPoolAttempts { .. }
            | Self::ModelRoutingLive { .. }
            | Self::UpstreamAccountAttemptsWindow { .. } => descriptor_runtime_topics(self),
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
            Self::UpstreamAccountAttemptsWindow { .. } => "upstream-account-attempts.window",
        }
    }

    fn schema_epoch(&self) -> String {
        match self {
            Self::AppVersion => "app.version/v1".to_string(),
            Self::QuotaCurrent => "quota.current/v1".to_string(),
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

fn descriptor_history_and_cache(topic: &SubscriptionTopic) -> SubscriptionTopicDescriptor {
    match topic {
        SubscriptionTopic::InvocationHistoryWindow { scope }
        | SubscriptionTopic::InvocationHistoryOverview { scope }
        | SubscriptionTopic::PromptCacheConversationBindingCurrent { scope } => {
            descriptor_scope_topic(topic, scope.descriptor_params())
        }
        SubscriptionTopic::PromptCacheConversationOperationsWindow { scope, info_type } => {
            descriptor_scope_with_info(topic, scope, info_type)
        }
        SubscriptionTopic::PromptCacheWindow {
            selection,
            detail_level,
            recent_invocation_limit,
        } => descriptor_prompt_cache_window(
            topic,
            *selection,
            detail_level,
            *recent_invocation_limit,
        ),
        SubscriptionTopic::PromptCacheStickyWindow {
            account_id,
            selection,
        } => descriptor_prompt_cache_sticky(topic, *account_id, *selection),
        _ => unreachable!("history and cache descriptor family"),
    }
}

fn descriptor_runtime_topics(topic: &SubscriptionTopic) -> SubscriptionTopicDescriptor {
    match topic {
        SubscriptionTopic::SummaryCurrent {
            window,
            time_zone,
            limit,
            upstream_account_id,
        } => descriptor_summary(topic, window, time_zone, *limit, *upstream_account_id),
        SubscriptionTopic::TimeseriesOpenWindow {
            range,
            time_zone,
            bucket,
            settlement_hour,
            upstream_account_id,
        } => descriptor_timeseries(
            topic,
            range,
            time_zone,
            bucket,
            *settlement_hour,
            *upstream_account_id,
        ),
        SubscriptionTopic::ParallelWorkCurrent {
            range,
            time_zone,
            bucket,
            upstream_account_id,
        } => descriptor_parallel_work(topic, range, time_zone, bucket, *upstream_account_id),
        SubscriptionTopic::ForwardProxyLive => empty_subscription_descriptor(topic),
        SubscriptionTopic::InvocationPoolAttempts { invoke_id } => SubscriptionTopicDescriptor {
            topic: topic.name().to_string(),
            params: BTreeMap::from([("invokeId".to_string(), invoke_id.clone())]),
        },
        SubscriptionTopic::ModelRoutingLive {
            window,
            model,
            state,
            limit,
        } => descriptor_model_routing(topic, window, model, state, *limit),
        SubscriptionTopic::UpstreamAccountAttemptsWindow {
            account_id,
            page,
            page_size,
            attempt_type,
            model,
            sticky_key,
        } => descriptor_upstream_attempts(
            topic,
            *account_id,
            *page,
            *page_size,
            attempt_type,
            model,
            sticky_key,
        ),
        _ => unreachable!("runtime descriptor family"),
    }
}

fn empty_subscription_descriptor(topic: &SubscriptionTopic) -> SubscriptionTopicDescriptor {
    SubscriptionTopicDescriptor {
        topic: topic.name().to_string(),
        params: BTreeMap::new(),
    }
}
