fn apply_dashboard_activity_slices(
    response: &mut DashboardActivityResponse,
    current: Option<&DashboardCurrentProjectionSlice>,
    network: Option<&DashboardNetworkProjectionSlice>,
) {
    apply_dashboard_current_slice(response, current, network);

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

fn apply_dashboard_current_slice(
    response: &mut DashboardActivityResponse,
    current: Option<&DashboardCurrentProjectionSlice>,
    network: Option<&DashboardNetworkProjectionSlice>,
) {
    let Some(current) = current else { return };
    response.live_revision = current.revision;
    response.summary.stats.in_progress_conversation_count =
        Some(current.in_progress_invocation_count);
    response.summary.stats.in_progress_retry_conversation_count =
        Some(current.retry_invocation_count);
    response.summary.stats.in_progress_avg_wait_ms = (current.in_progress_wait_sample_count > 0)
        .then_some(current.in_progress_wait_sum_ms / current.in_progress_wait_sample_count as f64);
    response.summary.stats.in_progress_phase_counts = Some(current.in_progress_phase_counts);
    let exact_range = dashboard_activity_response_exact_range(response);
    let model_performance_available = response.summary.model_performance.available;
    let Some(accounts) = response.accounts.as_mut() else {
        return;
    };
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
    let Some(exact_range) = exact_range else {
        return;
    };
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

pub(crate) fn apply_dashboard_terminal_slice_to_summary_response(
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
            // Replayed durable rows deliberately use sequence zero because their original
            // process-local sequence is not trustworthy after restart.  Their full source
            // identity is the dedupe key, so they must still be applied to range responses.
            || (delta.terminal_sequence != 0 && delta.terminal_sequence <= *terminal_sequence)
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
            | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
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
                    let expiry_result = if matches!(
                        &topic,
                        SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    ) {
                        hub.expire_dashboard_working_conversations_projection(state.clone(), &topic_key)
                            .await
                    } else {
                        hub.expire_prompt_cache_topic_window(&topic_key).await
                    };
                    if let Err(err) = expiry_result {
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
    hub.mark_internal_broadcast_listener_started();
    let listener_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                item = receiver.recv() => {
                    match item {
                        Ok(payload) => hub.handle_internal_broadcast(listener_state.clone(), payload).await,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            hub.mark_internal_broadcast_gap_and_recover(
                                listener_state.clone(),
                                skipped,
                            )
                            .await;
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
    let decoded = decode_subscription_stream_topics(&query)?;
    let preparation = prepare_subscription_stream(&state, decoded, &query).await?;
    let TopicSsePreparation {
        selected_topic_keys,
        selected_dashboard_topology_topic_names,
        topic_lease,
        prepared,
        server_push_lease,
    } = preparation;
    let mut live_receiver = state.subscription_hub.subscribe();
    #[cfg(test)]
    let dashboard_topology_observer_attempt = (query.reason.as_deref()
        == Some(DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON))
    .then_some(query.attempt)
    .flatten();
    let dashboard_topology_hub = state.subscription_hub.clone();
    let PreparedSubscriptionConnection {
        initial,
        last_sent_cursors: last_seen_by_topic,
        outcomes: _,
    } = prepared;
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

struct TopicSsePreparation {
    selected_topic_keys: HashSet<String>,
    selected_dashboard_topology_topic_names: Vec<String>,
    topic_lease: TopicSubscriptionLease,
    prepared: PreparedSubscriptionConnection,
    server_push_lease: ServerPushTopicLease,
}

async fn prepare_subscription_stream(
    state: &Arc<AppState>,
    decoded: DecodedSubscriptionStreamTopics,
    query: &SubscriptionStreamQuery,
) -> Result<TopicSsePreparation, ApiError> {
    let DecodedSubscriptionStreamTopics {
        descriptors,
        resume,
        selected_topics,
        selected_topic_keys,
        selected_dashboard_topology_topic_names,
    } = decoded;
    let resume_count = resume.len();
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
    Ok(TopicSsePreparation {
        selected_topic_keys,
        selected_dashboard_topology_topic_names,
        topic_lease,
        prepared,
        server_push_lease,
    })
}

struct DecodedSubscriptionStreamTopics {
    descriptors: Vec<SubscriptionTopicDescriptor>,
    resume: Vec<SubscriptionResumeCursor>,
    selected_topics: Vec<SubscriptionTopic>,
    selected_topic_keys: HashSet<String>,
    selected_dashboard_topology_topic_names: Vec<String>,
}

fn decode_subscription_stream_topics(
    query: &SubscriptionStreamQuery,
) -> Result<DecodedSubscriptionStreamTopics, ApiError> {
    let descriptors = decode_topics_query(query.topics.as_deref())?;
    let resume = decode_resume_query(query.resume.as_deref(), &descriptors)?;
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
        .collect();
    Ok(DecodedSubscriptionStreamTopics {
        descriptors,
        resume,
        selected_topics,
        selected_topic_keys,
        selected_dashboard_topology_topic_names,
    })
}
