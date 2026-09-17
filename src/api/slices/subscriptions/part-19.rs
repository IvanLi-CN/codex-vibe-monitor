async fn build_cached_dashboard_activity(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    recent_limit: i64,
    include_accounts: bool,
    include_recent: bool,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let base = build_dashboard_activity_topic_materialized_base(
        state.as_ref(),
        &DashboardActivityQuery {
            range: range.to_string(),
            recent_limit: Some(recent_limit),
            time_zone: Some(time_zone.to_string()),
            include_accounts,
            include_recent: Some(include_recent),
        },
    )
    .await?;
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::Activity {
            base: Arc::new(StdMutex::new(DashboardActivityMaterializerState::new(base))),
            reporting_tz,
            source_scope,
        },
    ))
}

struct CachedSummaryPayloadContext {
    query: SummaryQuery,
    summary_window: SummaryWindow,
    reporting_tz: Tz,
    response_limit: i64,
    upstream_account_id: Option<i64>,
    projection: Arc<SummaryProjection>,
    pending_terminal_deltas: Vec<DashboardActivityTerminalDelta>,
    initial_terminal_slice_suppressions: HashSet<u64>,
    gaps: Vec<DeltaGapProof>,
}

async fn build_cached_summary(
    state: Arc<AppState>,
    window: &str,
    time_zone: &str,
    limit: Option<i64>,
    upstream_account_id: Option<i64>,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let query = SummaryQuery {
        window: Some(window.to_string()),
        limit,
        time_zone: Some(time_zone.to_string()),
        upstream_account_id,
    };
    let summary_window = parse_summary_window(&query, state.config.list_limit_max as i64)?;
    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
    let Some((projection, pending_terminal_deltas, initial_terminal_slice_suppressions, gaps)) =
        state
            .subscription_hub
            .summary_projection_with_terminal_overlay(
                matches!(&summary_window, SummaryWindow::All),
                upstream_account_id,
            )
            .await?
    else {
        return Err(ApiError::unavailable(anyhow!(
            "summary projection has not completed hydration"
        )));
    };
    build_cached_summary_from_projection(CachedSummaryPayloadContext {
        query,
        summary_window,
        reporting_tz,
        response_limit: state.config.list_limit_max as i64,
        upstream_account_id,
        projection,
        pending_terminal_deltas,
        initial_terminal_slice_suppressions,
        gaps,
    })
}

fn build_cached_summary_from_projection(
    context: CachedSummaryPayloadContext,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let CachedSummaryPayloadContext {
        query,
        summary_window,
        reporting_tz,
        response_limit,
        upstream_account_id,
        projection,
        pending_terminal_deltas,
        initial_terminal_slice_suppressions,
        gaps,
    } = context;
    if crate::summary_delta_gap_affects_selection(
        projection.as_ref(),
        &gaps,
        &pending_terminal_deltas,
        &summary_window,
        reporting_tz,
        upstream_account_id,
    ) {
        return Err(ApiError::unavailable(anyhow!(
            "summary delta journal has an unproven change for the requested selection"
        )));
    }
    let mut response = projection.response_for_query_with_rolling_delta(
        &query,
        response_limit,
        !matches!(summary_window, SummaryWindow::All)
            && crate::summary_delta_affects_selection(
                projection.as_ref(),
                &pending_terminal_deltas,
                &summary_window,
                reporting_tz,
                upstream_account_id,
            ),
    )?;
    if let SummaryWindow::Current(limit) = &summary_window {
        projection.apply_rolling_delta_to_current_response(
            &mut response,
            *limit,
            upstream_account_id,
            &pending_terminal_deltas,
        )?;
    } else {
        let mut replayed_terminal_sequence = 0;
        apply_dashboard_terminal_slice_to_summary_response(
            &mut response,
            &mut replayed_terminal_sequence,
            &summary_window,
            reporting_tz,
            InvocationSourceScope::All,
            upstream_account_id,
            &DashboardTerminalProjectionSlice {
                revision: 0,
                deltas: pending_terminal_deltas,
            },
        );
    }
    let range_start =
        summary_window_range(&summary_window, reporting_tz, Utc::now())?.map(|(start, _)| start);
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::Summary {
            base: Arc::new(StdMutex::new(
                DashboardSummaryMaterializerState::from_summary_projection(
                    response,
                    initial_terminal_slice_suppressions,
                    range_start,
                ),
            )),
            window: summary_window,
            reporting_tz,
            source_scope: InvocationSourceScope::All,
            upstream_account_id,
        },
    ))
}

async fn build_cached_timeseries(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    bucket: &Option<String>,
    settlement_hour: Option<u8>,
    upstream_account_id: Option<i64>,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let base = TimeseriesTopicMaterializedBase::build(
        state.as_ref(),
        &TimeseriesQuery {
            range: range.to_string(),
            bucket: bucket.clone(),
            settlement_hour,
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        },
    )
    .await?;
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::Timeseries {
            base: Arc::new(StdMutex::new(base)),
            runtime: state.proxy_runtime_invocations.clone(),
        },
    ))
}

async fn build_cached_network_timeseries(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    upstream_account_id: Option<i64>,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let Json(response) = fetch_dashboard_network_timeseries(
        State(state),
        Query(DashboardNetworkTimeseriesQuery {
            range: range.to_string(),
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        }),
    )
    .await?;
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::NetworkTimeseries {
            base: Arc::new(response),
            upstream_account_id,
        },
    ))
}

async fn build_cached_network_recent(
    state: Arc<AppState>,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let Json(response) = fetch_dashboard_network_recent(
        State(state),
        Query(DashboardRecentNetworkWindowQuery::default()),
    )
    .await?;
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::NetworkRecent {
            base: Arc::new(response),
        },
    ))
}

async fn build_cached_parallel_work(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    bucket: &Option<String>,
    upstream_account_id: Option<i64>,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let base = build_dashboard_parallel_work_materializer_state(
        &state,
        ParallelWorkStatsQuery {
            range: range.to_string(),
            bucket: bucket.clone(),
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        },
    )
    .await?;
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::ParallelWork {
            base: Arc::new(StdMutex::new(base)),
        },
    ))
}

async fn build_cached_working_conversations(
    state: Arc<AppState>,
    page_size: i64,
    recent_invocation_limit: i64,
    blocked_binding_upstream_account_id: Option<i64>,
    blocked_binding_constraint_source: Option<BlockedBindingConstraintSource>,
) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
    let blocked_binding_filter = PromptCacheConversationBlockedBindingFilter {
        upstream_account_id: blocked_binding_upstream_account_id,
        constraint_source: blocked_binding_constraint_source,
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
            recent_invocation_limit: Some(recent_invocation_limit),
            page_size: Some(page_size),
            cursor: None,
            snapshot_at: None,
            blocked_binding_filter: blocked_binding_filter.clone(),
        },
    )
    .await?;
    Ok(BuiltSubscriptionTopicPayload::Dashboard(
        DashboardTopicMaterializer::WorkingConversations {
            state: Arc::new(StdMutex::new(
                DashboardWorkingConversationsMaterializerState::new(
                    response,
                    page_size,
                    recent_invocation_limit,
                    blocked_binding_filter,
                ),
            )),
        },
    ))
}

impl SubscriptionTopic {
    async fn build_cached_payload(
        &self,
        state: Arc<AppState>,
    ) -> Result<BuiltSubscriptionTopicPayload, ApiError> {
        if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto
            && let Some(payload) = self.build_cached_dashboard_payload(state.clone()).await?
        {
            return Ok(payload);
        }
        Ok(BuiltSubscriptionTopicPayload::Json(
            self.build_payload(state).await?,
        ))
    }

    async fn build_cached_dashboard_payload(
        &self,
        state: Arc<AppState>,
    ) -> Result<Option<BuiltSubscriptionTopicPayload>, ApiError> {
        match self {
            Self::DashboardActivityCurrent {
                range,
                time_zone,
                recent_limit,
                include_accounts,
                include_recent,
            } if range != "yesterday" => Ok(Some(
                build_cached_dashboard_activity(
                    state,
                    range,
                    time_zone,
                    *recent_limit,
                    *include_accounts,
                    *include_recent,
                )
                .await?,
            )),
            Self::SummaryCurrent {
                window,
                time_zone,
                limit,
                upstream_account_id,
            } => Ok(Some(
                build_cached_summary(state, window, time_zone, *limit, *upstream_account_id)
                    .await?,
            )),
            Self::TimeseriesOpenWindow {
                range,
                time_zone,
                bucket,
                settlement_hour,
                upstream_account_id,
            } if range != "yesterday" => Ok(Some(
                build_cached_timeseries(
                    state,
                    range,
                    time_zone,
                    bucket,
                    *settlement_hour,
                    *upstream_account_id,
                )
                .await?,
            )),
            Self::DashboardNetworkTimeseriesWindow {
                range,
                time_zone,
                upstream_account_id,
            } => Ok(Some(
                build_cached_network_timeseries(state, range, time_zone, *upstream_account_id)
                    .await?,
            )),
            Self::DashboardNetworkRecentCurrent => {
                Ok(Some(build_cached_network_recent(state).await?))
            }
            Self::ParallelWorkCurrent {
                range,
                time_zone,
                bucket,
                upstream_account_id,
            } if range != "yesterday" => Ok(Some(
                build_cached_parallel_work(state, range, time_zone, bucket, *upstream_account_id)
                    .await?,
            )),
            Self::DashboardWorkingConversationsCurrent {
                page_size,
                recent_invocation_limit,
                blocked_binding_upstream_account_id,
                blocked_binding_constraint_source,
            } => Ok(Some(
                build_cached_working_conversations(
                    state,
                    *page_size,
                    *recent_invocation_limit,
                    *blocked_binding_upstream_account_id,
                    *blocked_binding_constraint_source,
                )
                .await?,
            )),
            _ => Ok(None),
        }
    }

    async fn build_payload(&self, state: Arc<AppState>) -> Result<Value, ApiError> {
        match self {
            Self::AppVersion
            | Self::QuotaCurrent
            | Self::DashboardNetworkRecentCurrent
            | Self::ForwardProxyLive
            | Self::InvocationPoolAttempts { .. } => {
                build_simple_subscription_payload(self, state).await
            }
            Self::DashboardActivityCurrent { .. }
            | Self::DashboardNetworkTimeseriesWindow { .. }
            | Self::DashboardWorkingConversationsCurrent { .. } => {
                build_dashboard_json_payload(self, state).await
            }
            Self::InvocationWindow {
                limit,
                model,
                status,
            } => build_invocation_window_payload(state, *limit, model, status).await,
            Self::InvocationHistoryWindow { scope }
            | Self::InvocationHistoryOverview { scope }
            | Self::PromptCacheConversationBindingCurrent { scope } => {
                build_history_payload(state, self, scope).await
            }
            Self::PromptCacheConversationOperationsWindow { scope, info_type } => {
                build_operations_payload(state, scope, info_type).await
            }
            Self::PromptCacheWindow {
                selection,
                detail_level,
                recent_invocation_limit,
            } => {
                build_prompt_cache_window_payload(
                    state,
                    *selection,
                    detail_level,
                    *recent_invocation_limit,
                )
                .await
            }
            Self::PromptCacheStickyWindow {
                account_id,
                selection,
            } => build_prompt_cache_sticky_payload(state, *account_id, *selection).await,
            Self::SummaryCurrent {
                window,
                time_zone,
                limit,
                upstream_account_id,
            } => {
                build_summary_payload(state, window, time_zone, *limit, *upstream_account_id).await
            }
            Self::TimeseriesOpenWindow {
                range,
                time_zone,
                bucket,
                settlement_hour,
                upstream_account_id,
            } => {
                build_timeseries_payload(
                    state,
                    range,
                    time_zone,
                    bucket,
                    *settlement_hour,
                    *upstream_account_id,
                )
                .await
            }
            Self::ParallelWorkCurrent {
                range,
                time_zone,
                bucket,
                upstream_account_id,
            } => {
                build_parallel_work_payload(state, range, time_zone, bucket, *upstream_account_id)
                    .await
            }
            Self::ModelRoutingLive {
                window,
                model,
                state: route_state,
                limit,
            } => build_model_routing_payload(state, window, model, route_state, *limit).await,
            Self::UpstreamAccountAttemptsWindow {
                account_id,
                page,
                page_size,
                attempt_type,
                model,
                sticky_key,
            } => {
                build_upstream_attempts_payload(
                    state,
                    *account_id,
                    *page,
                    *page_size,
                    attempt_type,
                    model,
                    sticky_key,
                )
                .await
            }
        }
    }
}

async fn build_simple_subscription_payload(
    topic: &SubscriptionTopic,
    state: Arc<AppState>,
) -> Result<Value, ApiError> {
    let value = match topic {
        SubscriptionTopic::AppVersion => {
            let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
            serde_json::to_value(VersionResponse { backend, frontend })?
        }
        SubscriptionTopic::QuotaCurrent => {
            let Json(snapshot) = latest_quota_snapshot(State(state)).await?;
            serde_json::to_value(snapshot)?
        }
        SubscriptionTopic::DashboardNetworkRecentCurrent => {
            let Json(response) = fetch_dashboard_network_recent(
                State(state),
                Query(DashboardRecentNetworkWindowQuery::default()),
            )
            .await?;
            serde_json::to_value(response)?
        }
        SubscriptionTopic::ForwardProxyLive => {
            let Json(response) = fetch_forward_proxy_live_stats(State(state)).await?;
            serde_json::to_value(response)?
        }
        SubscriptionTopic::InvocationPoolAttempts { invoke_id } => {
            let Json(response) =
                fetch_invocation_pool_attempts(State(state), AxumPath(invoke_id.clone())).await?;
            serde_json::to_value(response)?
        }
        _ => unreachable!("simple subscription payload family"),
    };
    Ok(value)
}

async fn build_dashboard_json_payload(
    topic: &SubscriptionTopic,
    state: Arc<AppState>,
) -> Result<Value, ApiError> {
    match topic {
        SubscriptionTopic::DashboardActivityCurrent {
            range,
            time_zone,
            recent_limit,
            include_accounts,
            include_recent,
        } => {
            build_dashboard_activity_payload(
                state,
                range,
                time_zone,
                *recent_limit,
                *include_accounts,
                *include_recent,
            )
            .await
        }
        SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            range,
            time_zone,
            upstream_account_id,
        } => build_network_timeseries_payload(state, range, time_zone, *upstream_account_id).await,
        SubscriptionTopic::DashboardWorkingConversationsCurrent {
            page_size,
            recent_invocation_limit,
            blocked_binding_upstream_account_id,
            blocked_binding_constraint_source,
        } => {
            build_working_conversations_payload(
                state,
                *page_size,
                *recent_invocation_limit,
                *blocked_binding_upstream_account_id,
                *blocked_binding_constraint_source,
            )
            .await
        }
        _ => unreachable!("dashboard subscription payload family"),
    }
}

async fn build_dashboard_activity_payload(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    recent_limit: i64,
    include_accounts: bool,
    include_recent: bool,
) -> Result<Value, ApiError> {
    let Json(response) = fetch_dashboard_activity(
        State(state),
        Query(DashboardActivityQuery {
            range: range.to_string(),
            recent_limit: Some(recent_limit),
            time_zone: Some(time_zone.to_string()),
            include_accounts,
            include_recent: Some(include_recent),
        }),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_network_timeseries_payload(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    upstream_account_id: Option<i64>,
) -> Result<Value, ApiError> {
    let Json(response) = fetch_dashboard_network_timeseries(
        State(state),
        Query(DashboardNetworkTimeseriesQuery {
            range: range.to_string(),
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        }),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_working_conversations_payload(
    state: Arc<AppState>,
    page_size: i64,
    recent_invocation_limit: i64,
    blocked_binding_upstream_account_id: Option<i64>,
    blocked_binding_constraint_source: Option<BlockedBindingConstraintSource>,
) -> Result<Value, ApiError> {
    let blocked_binding_constraint_source =
        blocked_binding_constraint_source.map(|value| match value {
            BlockedBindingConstraintSource::UpstreamAccountBinding => "upstreamAccountBinding",
            BlockedBindingConstraintSource::EncryptedSessionOwner => "encryptedSessionOwner",
        });
    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES),
            page_size: Some(page_size),
            cursor: None,
            snapshot_at: None,
            detail: Some("full".to_string()),
            recent_invocation_limit: Some(recent_invocation_limit),
            blocked_binding_upstream_account_id,
            blocked_binding_constraint_source: blocked_binding_constraint_source
                .map(str::to_string),
        }),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_invocation_window_payload(
    state: Arc<AppState>,
    limit: i64,
    model: &Option<String>,
    status: &Option<String>,
) -> Result<Value, ApiError> {
    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(limit),
            page: Some(1),
            page_size: Some(limit),
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

async fn build_history_payload(
    state: Arc<AppState>,
    topic: &SubscriptionTopic,
    scope: &ConversationSubscriptionScope,
) -> Result<Value, ApiError> {
    match topic {
        SubscriptionTopic::InvocationHistoryWindow { .. } => {
            let Json(response) = list_invocations(
                State(state),
                Query(scope.list_query(1, SUBSCRIPTION_CONVERSATION_HISTORY_LIMIT, None)),
            )
            .await?;
            Ok(serde_json::to_value(response)?)
        }
        SubscriptionTopic::InvocationHistoryOverview { .. } => {
            let overview = fetch_invocation_history_overview_with_runtime_overlay(
                state.clone(),
                scope.list_query(1, SUBSCRIPTION_CONVERSATION_HISTORY_LIMIT, None),
                runtime_overlay_snapshot(state.as_ref()),
                SUBSCRIPTION_CONVERSATION_OVERVIEW_MAX_RECORDS,
            )
            .await?;
            Ok(serde_json::to_value(overview)?)
        }
        SubscriptionTopic::PromptCacheConversationBindingCurrent { .. } => {
            Ok(serde_json::to_value(
                load_prompt_cache_conversation_binding_response_for_key(
                    state.as_ref(),
                    scope.binding_key().to_string(),
                )
                .await?,
            )?)
        }
        _ => unreachable!("history subscription payload family"),
    }
}

async fn build_operations_payload(
    state: Arc<AppState>,
    scope: &ConversationSubscriptionScope,
    info_type: &Option<String>,
) -> Result<Value, ApiError> {
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

async fn build_prompt_cache_window_payload(
    state: Arc<AppState>,
    selection: PromptCacheConversationSelection,
    detail_level: &PromptCacheConversationDetailLevel,
    recent_invocation_limit: Option<i64>,
) -> Result<Value, ApiError> {
    let (limit, activity_hours, activity_minutes) = match selection {
        PromptCacheConversationSelection::Count(limit) => (Some(limit), None, None),
        PromptCacheConversationSelection::ActivityWindowHours(hours) => (None, Some(hours), None),
        PromptCacheConversationSelection::ActivityWindowMinutes(minutes) => {
            (None, None, Some(minutes))
        }
    };
    let detail = match detail_level {
        PromptCacheConversationDetailLevel::Full => "full",
        PromptCacheConversationDetailLevel::Compact => "compact",
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
            detail: Some(detail.to_string()),
            recent_invocation_limit,
            blocked_binding_upstream_account_id: None,
            blocked_binding_constraint_source: None,
        }),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_prompt_cache_sticky_payload(
    state: Arc<AppState>,
    account_id: i64,
    selection: AccountStickyKeySelection,
) -> Result<Value, ApiError> {
    Ok(serde_json::to_value(
        build_account_sticky_keys_response(&state.pool, account_id, selection)
            .await
            .map_err(ApiError::from)?,
    )?)
}

async fn build_summary_payload(
    state: Arc<AppState>,
    window: &str,
    time_zone: &str,
    limit: Option<i64>,
    upstream_account_id: Option<i64>,
) -> Result<Value, ApiError> {
    let response = load_summary_response_from_query(
        state.as_ref(),
        &SummaryQuery {
            window: Some(window.to_string()),
            limit,
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        },
        SummaryBuildRoute::Topic,
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_timeseries_payload(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    bucket: &Option<String>,
    settlement_hour: Option<u8>,
    upstream_account_id: Option<i64>,
) -> Result<Value, ApiError> {
    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: range.to_string(),
            bucket: bucket.clone(),
            settlement_hour,
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        }),
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_parallel_work_payload(
    state: Arc<AppState>,
    range: &str,
    time_zone: &str,
    bucket: &Option<String>,
    upstream_account_id: Option<i64>,
) -> Result<Value, ApiError> {
    let response = load_parallel_work_stats_response(
        &state,
        ParallelWorkStatsQuery {
            range: range.to_string(),
            bucket: bucket.clone(),
            time_zone: Some(time_zone.to_string()),
            upstream_account_id,
        },
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}

async fn build_model_routing_payload(
    state: Arc<AppState>,
    window: &str,
    model: &Option<String>,
    route_state: &Option<String>,
    limit: i64,
) -> Result<Value, ApiError> {
    let Json(response) = get_model_routing_live(
        State(state),
        Query(ModelRoutingLiveQuery {
            window: Some(window.to_string()),
            model: model.clone(),
            state: route_state.clone(),
            limit: Some(limit as usize),
        }),
    )
    .await
    .map_err(|(_status, message)| ApiError::bad_request(anyhow!(message)))?;
    Ok(serde_json::to_value(response)?)
}

async fn build_upstream_attempts_payload(
    state: Arc<AppState>,
    account_id: i64,
    page: usize,
    page_size: usize,
    attempt_type: &Option<String>,
    model: &Option<String>,
    sticky_key: &Option<String>,
) -> Result<Value, ApiError> {
    let response = load_upstream_account_attempt_page_from_query(
        state.as_ref(),
        account_id,
        &ListUpstreamAccountAttemptsQuery {
            attempt_type: attempt_type.clone(),
            model: model.clone(),
            sticky_key: sticky_key.clone(),
            page: Some(page),
            page_size: Some(page_size),
        },
    )
    .await?;
    Ok(serde_json::to_value(response)?)
}
