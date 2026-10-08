use super::*;

impl SubscriptionTopic {
    pub(super) async fn build_cached_payload(
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
                } => {
                    let query = SummaryQuery {
                        window: Some(window.clone()),
                        limit: *limit,
                        time_zone: Some(time_zone.clone()),
                        upstream_account_id: *upstream_account_id,
                    };
                    let summary_window =
                        parse_summary_window(&query, state.config.list_limit_max as i64)?;
                    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
                    let projection_with_overlay = state
                        .subscription_hub
                        .summary_projection_with_terminal_overlay(
                            matches!(&summary_window, SummaryWindow::All),
                            *upstream_account_id,
                        )
                        .await?;
                    if let Some((
                        projection,
                        pending_terminal_deltas,
                        initial_terminal_slice_suppressions,
                        gaps,
                    )) = projection_with_overlay
                    {
                        if crate::summary_delta_gap_affects_selection(
                            projection.as_ref(),
                            &gaps,
                            &pending_terminal_deltas,
                            &summary_window,
                            reporting_tz,
                            *upstream_account_id,
                        ) {
                            return Err(ApiError::unavailable(anyhow!(
                                "summary delta journal has an unproven change for the requested selection"
                            )));
                        }
                        let mut response = projection.response_for_query_with_rolling_delta(
                            &query,
                            state.config.list_limit_max as i64,
                            !matches!(summary_window, SummaryWindow::All)
                                && crate::summary_delta_affects_selection(
                                    projection.as_ref(),
                                    &pending_terminal_deltas,
                                    &summary_window,
                                    reporting_tz,
                                    *upstream_account_id,
                                ),
                        )?;
                        let current_selection =
                            if let SummaryWindow::Current(limit) = &summary_window {
                                projection.apply_rolling_delta_to_current_response(
                                    &mut response,
                                    *limit,
                                    *upstream_account_id,
                                    &pending_terminal_deltas,
                                )?;
                                true
                            } else {
                                false
                            };
                        // The immutable SummaryProjection represents one durable baseline. A
                        // terminal remains in the hub-owned overlay until that exact projection
                        // contains its durable identity, including after SQLite ACK but before a
                        // background projection swap. This stays entirely in memory.
                        let mut replayed_terminal_sequence = 0;
                        if !current_selection {
                            apply_dashboard_terminal_slice_to_summary_response(
                                &mut response,
                                &mut replayed_terminal_sequence,
                                &summary_window,
                                reporting_tz,
                                InvocationSourceScope::All,
                                *upstream_account_id,
                                &DashboardTerminalProjectionSlice {
                                    revision: 0,
                                    deltas: pending_terminal_deltas,
                                },
                            );
                        }
                        let range_start =
                            summary_window_range(&summary_window, reporting_tz, Utc::now())?
                                .map(|(start, _)| start);
                        return Ok(BuiltSubscriptionTopicPayload::Dashboard(
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
                                upstream_account_id: *upstream_account_id,
                            },
                        ));
                    }

                    return Err(ApiError::unavailable(anyhow!(
                        "summary projection has not completed hydration"
                    )));
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

    pub(super) async fn build_payload(&self, state: Arc<AppState>) -> Result<Value, ApiError> {
        match self {
            Self::AppVersion => {
                let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
                Ok(serde_json::to_value(VersionResponse { backend, frontend })?)
            }
            Self::ManagedTaskCatalog => {
                let Json(tasks) = list_managed_tasks(State(state)).await?;
                Ok(serde_json::to_value(tasks)?)
            }
            Self::ManagedTaskRuntime => {
                let Json(snapshot) = get_managed_task_runtime(State(state)).await?;
                Ok(serde_json::to_value(snapshot)?)
            }
            Self::ManagedTaskTimeline => build_managed_task_timeline_topic_payload(None).await,
            Self::ManagedTaskTimelineV2 => {
                build_managed_task_timeline_revision_topic_payload().await
            }
            Self::ManagedTaskDetail { task_key } => {
                let Some(store) = crate::maintenance_store::global() else {
                    return Err(ApiError::unavailable(anyhow!(
                        "maintenance database unavailable"
                    )));
                };
                let Some(detail) = store.detail(task_key).await.map_err(ApiError::from)? else {
                    return Err(ApiError::bad_request(anyhow!("managed task not found")));
                };
                Ok(serde_json::to_value(detail)?)
            }
            Self::ManagedTaskWorkload {
                task_key,
                window_hours: _,
                limit,
            } => {
                let Some(store) = crate::maintenance_store::global() else {
                    return Err(ApiError::unavailable(anyhow!(
                        "maintenance database unavailable"
                    )));
                };
                let Some(trend) = store
                    .workload_window(task_key, *limit)
                    .await
                    .map_err(ApiError::from)?
                else {
                    return Err(ApiError::bad_request(anyhow!("managed task not found")));
                };
                Ok(serde_json::to_value(trend)?)
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
            Self::UpstreamAccountAttemptsWindow {
                account_id,
                page,
                page_size,
                attempt_type,
                model,
                sticky_key,
            } => {
                let response = load_upstream_account_attempt_page_from_query(
                    state.as_ref(),
                    *account_id,
                    &ListUpstreamAccountAttemptsQuery {
                        attempt_type: attempt_type.clone(),
                        model: model.clone(),
                        sticky_key: sticky_key.clone(),
                        page: Some(*page),
                        page_size: Some(*page_size),
                    },
                )
                .await?;
                Ok(serde_json::to_value(response)?)
            }
        }
    }
}
