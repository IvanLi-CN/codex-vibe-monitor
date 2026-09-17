pub(crate) fn overlay_dashboard_network_live_snapshot(
    mut snapshot: DashboardActivityLiveSnapshot,
    network_open_buckets: &HashMap<
        DashboardNetworkScopeKey,
        DashboardRuntimeNetworkOpenBucketBaseline,
    >,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
) -> DashboardActivityLiveSnapshot {
    let now = Utc::now();
    let account_rates = dashboard_network_speed_cache.snapshot_account_rates(now);
    let mut existing_account_keys = HashSet::new();
    for account in &mut snapshot.accounts {
        existing_account_keys.insert(account.account_key.clone());
        let rate = account_rates
            .get(&account.upstream_account_id)
            .copied()
            .unwrap_or_default();
        account.upload_bytes_per_second = rate.upload_bytes_per_second;
        account.download_bytes_per_second = rate.download_bytes_per_second;
        account.network_live_bucket = Some(dashboard_network_live_bucket_from_memory(
            dashboard_network_speed_cache,
            network_open_buckets,
            DashboardNetworkScopeKey::account_scope(account.upstream_account_id),
            now,
        ));
    }
    for (upstream_account_id, rate) in account_rates {
        let account_key = upstream_account_id
            .map(|id| format!("upstream:{id}"))
            .unwrap_or_else(|| "unassigned".to_string());
        if existing_account_keys.contains(&account_key) {
            continue;
        }
        snapshot.accounts.push(DashboardActivityLiveAccount {
            account_key,
            upstream_account_id,
            upstream_account_name: None,
            in_progress_invocation_count: 0,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            upload_bytes_per_second: rate.upload_bytes_per_second,
            download_bytes_per_second: rate.download_bytes_per_second,
            network_live_bucket: Some(dashboard_network_live_bucket_from_memory(
                dashboard_network_speed_cache,
                network_open_buckets,
                DashboardNetworkScopeKey::account_scope(upstream_account_id),
                now,
            )),
        });
    }
    snapshot
        .accounts
        .sort_by(|left, right| left.account_key.cmp(&right.account_key));
    snapshot.network_live_bucket = Some(dashboard_network_live_bucket_from_memory(
        dashboard_network_speed_cache,
        network_open_buckets,
        DashboardNetworkScopeKey::Global,
        now,
    ));
    snapshot.network_realtime_rate = Some(build_dashboard_network_realtime_rate_response(
        dashboard_network_speed_cache
            .snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, now),
    ));
    snapshot.generated_at = format_utc_iso(now);
    snapshot
}

fn dashboard_network_live_bucket_from_memory(
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    network_open_buckets: &HashMap<
        DashboardNetworkScopeKey,
        DashboardRuntimeNetworkOpenBucketBaseline,
    >,
    scope: DashboardNetworkScopeKey,
    now: DateTime<Utc>,
) -> DashboardNetworkTimeseriesPointResponse {
    let snapshot = dashboard_network_speed_cache.snapshot_open_bucket(scope, now);
    let totals = network_open_buckets
        .get(&scope)
        .filter(|baseline| {
            baseline.bucket_start == snapshot.bucket_start
                && baseline.bucket_end == snapshot.bucket_end
        })
        .map(|baseline| {
            let mut totals = baseline.baseline_totals;
            totals.add_assign(DashboardNetworkByteTotals {
                upload_bytes: snapshot
                    .totals
                    .upload_bytes
                    .saturating_sub(baseline.memory_totals_at_install.upload_bytes),
                download_bytes: snapshot
                    .totals
                    .download_bytes
                    .saturating_sub(baseline.memory_totals_at_install.download_bytes),
            });
            totals
        })
        .unwrap_or(snapshot.totals);
    build_dashboard_network_timeseries_point_response(
        snapshot.bucket_start,
        snapshot.bucket_end,
        totals,
        ExactUtcRange {
            start: snapshot.bucket_start,
            end: now.min(snapshot.bucket_end),
        },
        true,
    )
}

pub(crate) fn schedule_dashboard_activity_live_snapshot(state: &AppState) {
    if state.shutdown.is_cancelled() {
        return;
    }
    if let Err(err) = state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
    {
        state
            .proxy_runtime_invocations
            .mark_degraded("network_cache_bind_failed");
        warn!(
            ?err,
            "failed to bind dashboard network cache to runtime projection"
        );
        return;
    }
    state
        .proxy_runtime_invocations
        .mark_dashboard_dirty("dashboard_live_schedule");
    ensure_dashboard_activity_live_snapshot_producer(state);
}

pub(crate) fn schedule_dashboard_network_projection(state: &AppState) {
    if state.shutdown.is_cancelled() {
        return;
    }
    if let Err(err) = state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
    {
        state
            .proxy_runtime_invocations
            .mark_degraded("network_cache_bind_failed");
        warn!(
            ?err,
            "failed to bind dashboard network cache to runtime projection"
        );
        return;
    }
    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Auto {
        state
            .proxy_runtime_invocations
            .mark_dashboard_network_dirty();
    } else {
        state
            .proxy_runtime_invocations
            .mark_dashboard_dirty("dashboard_network_legacy_schedule");
    }
    ensure_dashboard_activity_live_snapshot_producer(state);
}

pub(crate) fn ensure_dashboard_activity_live_snapshot_producer(state: &AppState) {
    if state.shutdown.is_cancelled()
        || state
            .proxy_runtime_invocations
            .pending_dashboard_publish_window()
            .is_none()
        || (!state
            .proxy_runtime_invocations
            .has_pending_dashboard_terminal_publish()
            && !state
                .subscription_hub
                .has_active_dashboard_activity_live_topic_sync())
    {
        return;
    }
    let _ = state
        .dashboard_activity_live_broadcast_seq
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    if state
        .dashboard_activity_live_broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    state.proxy_runtime_invocations.set_producer_running(true);
    tokio::spawn(run_dashboard_activity_live_snapshot_producer(
        DashboardActivityLiveSnapshotProducer {
            latest_seq: state.dashboard_activity_live_broadcast_seq.clone(),
            broadcast_running: state.dashboard_activity_live_broadcast_running.clone(),
            pool: state.pool.clone(),
            proxy_runtime_invocations: state.proxy_runtime_invocations.clone(),
            dashboard_network_speed_cache: state.dashboard_network_speed_cache.clone(),
            subscription_hub: state.subscription_hub.clone(),
            broadcaster: state.broadcaster.clone(),
            shutdown: state.shutdown.clone(),
        },
    ));
}

struct DashboardActivityLiveSnapshotProducer {
    latest_seq: Arc<AtomicU64>,
    broadcast_running: Arc<AtomicBool>,
    pool: Pool<Sqlite>,
    proxy_runtime_invocations: Arc<RuntimeProjectionHub>,
    dashboard_network_speed_cache: Arc<DashboardNetworkSpeedCache>,
    subscription_hub: Arc<SubscriptionHub>,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    shutdown: CancellationToken,
}

async fn run_dashboard_activity_live_snapshot_producer(
    producer: DashboardActivityLiveSnapshotProducer,
) {
    run_dashboard_activity_live_snapshot_loop(producer).await;
}

async fn run_dashboard_activity_live_snapshot_loop(
    DashboardActivityLiveSnapshotProducer {
        latest_seq,
        broadcast_running,
        pool,
        proxy_runtime_invocations,
        dashboard_network_speed_cache,
        subscription_hub,
        broadcaster,
        shutdown,
    }: DashboardActivityLiveSnapshotProducer,
) {
    let mut delivered_seq = latest_seq.load(Ordering::Acquire).saturating_sub(1);
    loop {
        let Some(window) = proxy_runtime_invocations.pending_dashboard_publish_window() else {
            broadcast_running.store(false, Ordering::Release);
            proxy_runtime_invocations.set_producer_running(false);
            if proxy_runtime_invocations
                .pending_dashboard_publish_window()
                .is_some()
                && broadcast_running
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                proxy_runtime_invocations.set_producer_running(true);
                continue;
            }
            return;
        };
        tokio::select! {
            _ = shutdown.cancelled() => {
                broadcast_running.store(false, Ordering::Release);
                proxy_runtime_invocations.set_producer_running(false);
                return;
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(window.deadline)) => {}
            _ = proxy_runtime_invocations.wait_for_dashboard_publish_signal() => continue,
        }

        record_dashboard_projection_cadence_miss(&proxy_runtime_invocations, &window);

        let Some(window) = proxy_runtime_invocations.begin_dashboard_publish_window(window) else {
            continue;
        };

        let sent_seq = latest_seq.load(Ordering::Acquire);
        let has_active_subscribers = subscription_hub
            .has_active_dashboard_activity_live_topic()
            .await;
        publish_dashboard_projection_window(
            DashboardProjectionPublishContext {
                pool: &pool,
                runtime: &proxy_runtime_invocations,
                network_cache: &dashboard_network_speed_cache,
                broadcaster: &broadcaster,
            },
            window,
            has_active_subscribers,
            sent_seq,
            delivered_seq,
        )
        .await;
        complete_dashboard_projection_publish_window(
            proxy_runtime_invocations.as_ref(),
            window,
            has_active_subscribers,
        );
        delivered_seq = sent_seq;
    }
}

struct DashboardProjectionPublishContext<'a> {
    pool: &'a Pool<Sqlite>,
    runtime: &'a RuntimeProjectionHub,
    network_cache: &'a DashboardNetworkSpeedCache,
    broadcaster: &'a broadcast::Sender<BroadcastPayload>,
}

fn record_dashboard_projection_cadence_miss(
    runtime: &RuntimeProjectionHub,
    window: &DashboardProjectionPublishWindow,
) {
    let cadence = match window.slice {
        DashboardProjectionSlice::Current => DASHBOARD_RUNTIME_PROJECTION_COALESCE,
        DashboardProjectionSlice::Network => DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE,
        DashboardProjectionSlice::Terminal => DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE,
    };
    if Instant::now().saturating_duration_since(window.deadline) <= cadence {
        return;
    }
    match window.slice {
        DashboardProjectionSlice::Current => runtime.record_current_slice_cadence_miss(),
        DashboardProjectionSlice::Network => runtime.record_network_slice_cadence_miss(),
        DashboardProjectionSlice::Terminal => runtime.record_terminal_slice_cadence_miss(),
    }
}

async fn publish_dashboard_projection_window(
    context: DashboardProjectionPublishContext<'_>,
    window: DashboardProjectionPublishWindow,
    has_active_subscribers: bool,
    sent_seq: u64,
    delivered_seq: u64,
) {
    if !has_active_subscribers {
        if window.slice == DashboardProjectionSlice::Terminal {
            let _ = context.runtime.capture_terminal_slice();
        }
        return;
    }
    let started = Instant::now();
    match window.slice {
        DashboardProjectionSlice::Current => {
            publish_dashboard_current_slice(context, sent_seq, delivered_seq, started).await;
        }
        DashboardProjectionSlice::Network => publish_dashboard_network_slice(context),
        DashboardProjectionSlice::Terminal => publish_dashboard_terminal_slice(context),
    }
}

async fn publish_dashboard_current_slice(
    context: DashboardProjectionPublishContext<'_>,
    sent_seq: u64,
    delivered_seq: u64,
    started: Instant,
) {
    match capture_dashboard_activity_live_snapshot_from_runtime(
        context.pool,
        context.runtime,
        context.network_cache,
    )
    .await
    {
        Ok(capture) if capture.changed => {
            let revision = capture.snapshot.revision;
            let payload = match context.runtime.mode() {
                RuntimeProjectionMode::Auto => BroadcastPayload::DashboardCurrentSlice {
                    slice: Box::new(DashboardCurrentProjectionSlice::from(&capture.snapshot)),
                },
                RuntimeProjectionMode::Legacy => BroadcastPayload::DashboardActivityLive {
                    snapshot: Box::new(context.runtime.legacy_live_snapshot(capture.snapshot)),
                },
            };
            if let Err(err) = context.broadcaster.send(payload) {
                warn!(
                    ?err,
                    revision, "failed to broadcast dashboard current slice"
                );
            } else {
                tracing::debug!(
                    revision,
                    coalesced_mutation_count = sent_seq.saturating_sub(delivered_seq),
                    generated_to_sent_ms = started.elapsed().as_millis() as u64,
                    snapshot_origin = capture.snapshot_origin,
                    "broadcast dashboard current slice"
                );
            }
        }
        Ok(capture) => tracing::debug!(
            revision = capture.snapshot.revision,
            snapshot_origin = capture.snapshot_origin,
            "suppressed unchanged dashboard current slice"
        ),
        Err(err) => warn!(?err, "failed to capture dashboard current slice"),
    }
}

fn publish_dashboard_network_slice(context: DashboardProjectionPublishContext<'_>) {
    if context.runtime.mode() != RuntimeProjectionMode::Auto {
        return;
    }
    match context.runtime.capture_network_slice() {
        Ok(capture) if capture.changed => {
            let revision = capture.slice.revision;
            if let Err(err) = context
                .broadcaster
                .send(BroadcastPayload::DashboardNetworkSlice {
                    slice: Box::new(capture.slice),
                })
            {
                warn!(
                    ?err,
                    revision, "failed to broadcast dashboard network slice"
                );
            }
        }
        Ok(_) => tracing::debug!("suppressed unchanged dashboard network slice"),
        Err(err) => warn!(?err, "failed to capture dashboard network slice"),
    }
}

fn publish_dashboard_terminal_slice(context: DashboardProjectionPublishContext<'_>) {
    if context.runtime.mode() != RuntimeProjectionMode::Auto {
        return;
    }
    let Some(capture) = context.runtime.capture_terminal_slice() else {
        return;
    };
    let revision = capture.revision;
    if let Err(err) = context
        .broadcaster
        .send(BroadcastPayload::DashboardTerminalSlice {
            slice: Box::new(DashboardTerminalProjectionSlice {
                revision,
                deltas: capture.deltas,
            }),
        })
    {
        warn!(
            ?err,
            revision, "failed to broadcast dashboard terminal slice"
        );
    }
}

fn complete_dashboard_projection_publish_window(
    hub: &RuntimeProjectionHub,
    window: DashboardProjectionPublishWindow,
    has_active_subscribers: bool,
) {
    hub.complete_dashboard_publish_window(window);
    if has_active_subscribers && window.slice == DashboardProjectionSlice::Network {
        hub.mark_dashboard_network_dirty();
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum BroadcastPayload {
    Version {
        version: String,
    },
    // Test-only observer shims let pre-existing persistence tests assert their durable
    // side effects without reinstating complete records on the production runtime bus.
    #[cfg(test)]
    Records {
        records: Vec<ApiInvocation>,
    },
    #[cfg(test)]
    PromptCacheConversationChanged {
        prompt_cache_key: String,
    },
    #[cfg(test)]
    PromptCacheConversationStickyRouteChanged {
        sticky_key: String,
        previous_upstream_account_id: i64,
        upstream_account_id: i64,
    },
    DashboardActivityLive {
        snapshot: Box<DashboardActivityLiveSnapshot>,
    },
    DashboardCurrentSlice {
        slice: Box<DashboardCurrentProjectionSlice>,
    },
    DashboardNetworkSlice {
        slice: Box<DashboardNetworkProjectionSlice>,
    },
    DashboardTerminalSlice {
        #[serde(skip)]
        slice: Box<DashboardTerminalProjectionSlice>,
    },
    #[serde(rename = "pool_attempts")]
    PoolAttempts {
        invoke_id: String,
        attempts: Vec<ApiPoolUpstreamRequestAttempt>,
    },
    // This control event stays on the internal broadcaster. The producer could not determine
    // which account changed, so the subscription hub restores only active account topics.
    PoolAttemptsSnapshotUnavailable {
        invoke_id: String,
    },
    Quota {
        snapshot: Box<QuotaSnapshotResponse>,
    },
}

pub(crate) fn serialize_opt_finite_nonnegative_timing<S>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .serialize(serializer)
}

pub(crate) fn serialize_opt_finite_positive_timing<S>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    value
        .filter(|value| value.is_finite() && *value > 0.0)
        .serialize(serializer)
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiInvocation {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    #[sqlx(default)]
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) model: Option<String>,
    #[sqlx(default)]
    pub(crate) request_model: Option<String>,
    #[sqlx(default)]
    pub(crate) response_model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_input: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_cache_write: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_cache_read: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_output: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_reasoning: Option<f64>,
    #[sqlx(default)]
    pub(crate) cache_write_tokens: Option<i64>,
    pub(crate) status: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) live_phase: Option<String>,
    pub(crate) error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_status_code: Option<i64>,
    #[sqlx(default)]
    pub(crate) failure_kind: Option<String>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    #[sqlx(default)]
    #[serde(skip)]
    pub(crate) blocked_binding_json: Option<String>,
    #[sqlx(default)]
    pub(crate) stream_terminal_event: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_error_code: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_request_id: Option<String>,
    #[sqlx(default)]
    pub(crate) failure_class: Option<String>,
    #[sqlx(default)]
    pub(crate) is_actionable: Option<bool>,
    #[sqlx(default)]
    pub(crate) endpoint: Option<String>,
    #[sqlx(default)]
    pub(crate) compaction_request_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) compaction_response_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) image_intent: Option<String>,
    #[sqlx(default)]
    pub(crate) requester_ip: Option<String>,
    #[sqlx(default)]
    pub(crate) prompt_cache_key: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) sticky_key: Option<String>,
    #[sqlx(default)]
    pub(crate) route_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) response_content_encoding: Option<String>,
    #[sqlx(default)]
    pub(crate) request_compression_algorithm: Option<String>,
    #[sqlx(default)]
    pub(crate) transport: Option<String>,
    #[sqlx(default)]
    pub(crate) pool_attempt_count: Option<i64>,
    #[sqlx(default)]
    pub(crate) pool_distinct_account_count: Option<i64>,
    #[sqlx(default)]
    pub(crate) pool_attempt_terminal_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) requested_service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) billing_service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) proxy_weight_delta: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_estimated: Option<i64>,
    #[sqlx(default)]
    pub(crate) price_version: Option<String>,
    #[sqlx(default)]
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost_audit: Option<InvocationCostAudit>,
    #[sqlx(default)]
    pub(crate) request_raw_path: Option<String>,
    #[sqlx(default)]
    pub(crate) request_raw_size: Option<i64>,
    #[sqlx(default)]
    pub(crate) request_raw_truncated: Option<i64>,
    #[sqlx(default)]
    pub(crate) request_raw_truncated_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) response_raw_path: Option<String>,
    #[sqlx(default)]
    pub(crate) response_raw_size: Option<i64>,
    #[sqlx(default)]
    pub(crate) response_raw_truncated: Option<i64>,
    #[sqlx(default)]
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) detail_level: String,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) detail_pruned_at: Option<String>,
    #[sqlx(default)]
    pub(crate) detail_prune_reason: Option<String>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_total_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_req_read_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_req_parse_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_upstream_connect_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) first_token_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_positive_timing")]
    pub(crate) t_upstream_stream_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_resp_parse_ms: Option<f64>,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_persist_ms: Option<f64>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationCostAuditBreakdown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_write: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationCostAudit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recorded: Option<InvocationCostAuditBreakdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) local: Option<InvocationCostAuditBreakdown>,
    pub(crate) mismatch: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) absolute_diff_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recorded_price_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) local_price_version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListResponse {
    pub(crate) snapshot_id: i64,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) records: Vec<ApiInvocation>,
}
