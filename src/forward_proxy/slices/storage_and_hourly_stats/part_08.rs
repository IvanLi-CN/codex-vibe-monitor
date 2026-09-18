pub(crate) fn resolve_forward_proxy_endpoint_for_test(
    manager: &ForwardProxyManager,
    proxy_key: &str,
) -> Option<ForwardProxyEndpoint> {
    if proxy_key == FORWARD_PROXY_DIRECT_KEY {
        return Some(ForwardProxyEndpoint::direct());
    }
    manager
        .endpoints
        .iter()
        .find(|endpoint| endpoint.key == proxy_key)
        .cloned()
        .or_else(|| {
            manager
                .runtime
                .get(proxy_key)
                .and_then(|runtime| runtime.endpoint_url.as_deref())
                .and_then(parse_forward_proxy_entry)
                .map(|parsed| ForwardProxyEndpoint {
                    key: proxy_key.to_string(),
                    source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
                    display_name: parsed.display_name,
                    protocol: parsed.protocol,
                    endpoint_url: parsed.endpoint_url,
                    raw_url: Some(parsed.normalized),
                })
        })
}
pub(crate) async fn load_forward_proxy_endpoints_for_latency_test(
    state: &AppState,
    proxy_keys: &[String],
) -> Result<Vec<ForwardProxyEndpoint>, (StatusCode, String)> {
    let manager = state.forward_proxy.lock().await;
    let mut endpoints = Vec::with_capacity(proxy_keys.len());
    let mut missing = Vec::new();
    let mut seen = HashSet::new();
    for proxy_key in proxy_keys {
        let normalized = proxy_key.trim();
        if normalized.is_empty() || !seen.insert(normalized.to_string()) {
            continue;
        }
        match resolve_forward_proxy_endpoint_for_test(&manager, normalized) {
            Some(endpoint) => endpoints.push(endpoint),
            None => missing.push(normalized.to_string()),
        }
    }
    if endpoints.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "no proxy keys provided".to_string(),
        ));
    }
    if !missing.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("forward proxy node not found: {}", missing.join(", ")),
        ));
    }
    Ok(endpoints)
}

pub(crate) async fn timed_forward_proxy_egress_ip_probe(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
    client: &Client,
    request_timeout: Duration,
) -> ForwardProxyLatencyProbeTargetResult {
    let started = Instant::now();
    match fetch_forward_proxy_egress_ip(client, request_timeout).await {
        Ok(ip) => {
            if let Err(err) =
                persist_forward_proxy_egress_ip_result(&state.pool, selected_proxy, Some(&ip), None)
                    .await
            {
                warn!(
                    proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                    error = %err,
                    "failed to persist manual latency egress IP result"
                );
            }
            ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(elapsed_ms(started)),
                ip: Some(ip),
                http_status: None,
                error: None,
            }
        }
        Err(err) => {
            if let Err(persist_err) = persist_forward_proxy_egress_ip_result(
                &state.pool,
                selected_proxy,
                None,
                Some(&err.to_string()),
            )
            .await
            {
                warn!(
                    proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                    error = %persist_err,
                    "failed to persist manual latency egress IP failure"
                );
            }
            ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some(err.to_string()),
            }
        }
    }
}

pub(crate) async fn timed_forward_proxy_oauth_upstream_probe(
    client: &Client,
    request_timeout: Duration,
) -> ForwardProxyLatencyProbeTargetResult {
    let started = Instant::now();
    let target = match oauth_codex_latency_probe_target("models") {
        Ok(target) => target,
        Err(err) => {
            return ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some(err.to_string()),
            };
        }
    };

    let result = timeout(request_timeout, client.get(target).send()).await;
    match result {
        Ok(Ok(response)) => {
            let status = response.status();
            let ok = is_manual_latency_probe_reachable_status(status);
            ForwardProxyLatencyProbeTargetResult {
                ok,
                latency_ms: ok.then(|| elapsed_ms(started)),
                ip: None,
                http_status: Some(status.as_u16()),
                error: if ok {
                    None
                } else {
                    Some(format!("OAuth upstream returned status {status}"))
                },
            }
        }
        Ok(Err(err)) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(err.to_string()),
        },
        Err(_) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(format!(
                "manual latency test round timed out after {}s",
                timeout_seconds_for_message(request_timeout)
            )),
        },
    }
}

pub(crate) fn oauth_codex_latency_probe_target(path_segment: &str) -> Result<Url> {
    let mut target = oauth_bridge::oauth_codex_upstream_base_url()?;
    target.set_path(&format!(
        "{}/{}",
        target.path().trim_end_matches('/'),
        path_segment.trim_start_matches('/')
    ));
    Ok(target)
}

pub(crate) async fn timed_forward_proxy_codex_responses_probe(
    client: &Client,
    request_timeout: Duration,
) -> ForwardProxyLatencyProbeTargetResult {
    let started = Instant::now();
    let target = match oauth_codex_latency_probe_target("responses") {
        Ok(target) => target,
        Err(err) => {
            return ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some(err.to_string()),
            };
        }
    };

    let result = timeout(request_timeout, client.get(target).send()).await;
    match result {
        Ok(Ok(response)) => {
            let status = response.status();
            let ok = is_manual_latency_probe_reachable_status(status);
            ForwardProxyLatencyProbeTargetResult {
                ok,
                latency_ms: ok.then(|| elapsed_ms(started)),
                ip: None,
                http_status: Some(status.as_u16()),
                error: if ok {
                    None
                } else {
                    Some(format!("Codex responses upstream returned status {status}"))
                },
            }
        }
        Ok(Err(err)) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(err.to_string()),
        },
        Err(_) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(format!(
                "manual latency test round timed out after {}s",
                timeout_seconds_for_message(request_timeout)
            )),
        },
    }
}

pub(crate) fn failed_forward_proxy_latency_targets(
    egress_ip: &ForwardProxyLatencyProbeTargetResult,
    oauth_upstream: &ForwardProxyLatencyProbeTargetResult,
    codex_responses: &ForwardProxyLatencyProbeTargetResult,
) -> Vec<&'static str> {
    let mut targets = Vec::new();
    if !egress_ip.ok {
        targets.push(FORWARD_PROXY_LATENCY_TARGET_EGRESS_IP);
    }
    if !oauth_upstream.ok {
        targets.push(FORWARD_PROXY_LATENCY_TARGET_OAUTH_UPSTREAM);
    }
    if !codex_responses.ok {
        targets.push(FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES);
    }
    targets
}

pub(crate) fn forward_proxy_latency_target_timed_out(
    result: &ForwardProxyLatencyProbeTargetResult,
) -> bool {
    let Some(error) = result.error.as_deref() else {
        return false;
    };
    error.contains("timed out") || error.contains("budget exhausted")
}

pub(crate) fn is_manual_latency_probe_reachable_status(status: StatusCode) -> bool {
    status.as_u16() < 500
}

pub(crate) async fn run_forward_proxy_latency_test_round(
    state: Arc<AppState>,
    endpoint: &ForwardProxyEndpoint,
    accumulator: &mut ForwardProxyLatencyAccumulator,
    round: usize,
    single_started: Instant,
    is_single_node_test: bool,
) -> ForwardProxyLatencyTestNodeProgress {
    let selected_proxy = SelectedForwardProxy::from_endpoint(endpoint);
    let results = execute_forward_proxy_latency_round(
        state.as_ref(),
        endpoint,
        &selected_proxy,
        single_started,
        is_single_node_test,
    )
    .await;
    accumulator.record_round(
        &results.egress_ip,
        &results.oauth_upstream,
        &results.codex_responses,
    );
    record_forward_proxy_attempt(
        state,
        selected_proxy,
        results.is_success(),
        results
            .average_latency_ms()
            .or_else(|| Some(elapsed_ms(results.started))),
        if results.is_success() {
            None
        } else {
            Some(FORWARD_PROXY_FAILURE_SEND_ERROR)
        },
        true,
    )
    .await;
    forward_proxy_latency_round_progress(
        endpoint,
        accumulator,
        &results,
        round,
        single_started,
        is_single_node_test,
    )
}

struct ForwardProxyLatencyRoundResults {
    egress_ip: ForwardProxyLatencyProbeTargetResult,
    oauth_upstream: ForwardProxyLatencyProbeTargetResult,
    codex_responses: ForwardProxyLatencyProbeTargetResult,
    started: Instant,
}

impl ForwardProxyLatencyRoundResults {
    fn budget_exhausted() -> Self {
        let result = ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some("manual latency test budget exhausted".to_string()),
        };
        Self {
            egress_ip: result.clone(),
            oauth_upstream: result.clone(),
            codex_responses: result,
            started: Instant::now(),
        }
    }

    fn is_success(&self) -> bool {
        self.egress_ip.ok && self.oauth_upstream.ok && self.codex_responses.ok
    }

    fn average_latency_ms(&self) -> Option<f64> {
        let samples = [
            self.egress_ip.success_latency(),
            self.oauth_upstream.success_latency(),
            self.codex_responses.success_latency(),
        ];
        let (total, count) = samples
            .into_iter()
            .flatten()
            .fold((0.0, 0usize), |(total, count), sample| {
                (total + sample, count + 1)
            });
        (count > 0).then(|| total / count as f64)
    }
}

async fn execute_forward_proxy_latency_round(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    selected_proxy: &SelectedForwardProxy,
    single_started: Instant,
    is_single_node_test: bool,
) -> ForwardProxyLatencyRoundResults {
    let total_timeout = is_single_node_test
        .then(forward_proxy_manual_latency_single_timeout)
        .unwrap_or_else(|| Duration::from_secs(u64::MAX / 4));
    let Some(round_timeout) = remaining_timeout_budget(total_timeout, single_started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .map(|remaining| remaining.min(forward_proxy_manual_latency_round_timeout()))
    else {
        return ForwardProxyLatencyRoundResults::budget_exhausted();
    };
    match resolve_forward_proxy_probe_endpoint_url(
        state,
        endpoint,
        round_timeout,
        Some(&state.shutdown),
    )
    .await
    {
        Ok((endpoint_url, temporary_xray_key)) => {
            let results = match state
                .http_clients
                .client_for_forward_proxy(endpoint_url.as_ref())
            {
                Ok(client) => {
                    run_forward_proxy_latency_probes(state, selected_proxy, &client, round_timeout)
                        .await
                }
                Err(err) => forward_proxy_latency_round_error(err.to_string()),
            };
            if let Some(temp_key) = temporary_xray_key {
                let mut supervisor = state.xray_supervisor.lock().await;
                supervisor.remove_instance(&temp_key).await;
            }
            results
        }
        Err(err) => forward_proxy_latency_round_error(err.to_string()),
    }
}

async fn run_forward_proxy_latency_probes(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
    client: &Client,
    round_timeout: Duration,
) -> ForwardProxyLatencyRoundResults {
    let started = Instant::now();
    let egress_ip =
        timed_forward_proxy_egress_ip_probe(state, selected_proxy, client, round_timeout).await;
    let oauth_upstream = forward_proxy_latency_probe_with_remaining(
        client,
        round_timeout,
        started,
        timed_forward_proxy_oauth_upstream_probe,
    )
    .await;
    let codex_responses = forward_proxy_latency_probe_with_remaining(
        client,
        round_timeout,
        started,
        timed_forward_proxy_codex_responses_probe,
    )
    .await;
    ForwardProxyLatencyRoundResults {
        egress_ip,
        oauth_upstream,
        codex_responses,
        started,
    }
}

async fn forward_proxy_latency_probe_with_remaining<F, Fut>(
    client: &Client,
    round_timeout: Duration,
    started: Instant,
    probe: F,
) -> ForwardProxyLatencyProbeTargetResult
where
    F: FnOnce(&Client, Duration) -> Fut,
    Fut: std::future::Future<Output = ForwardProxyLatencyProbeTargetResult>,
{
    match remaining_timeout_budget(round_timeout, started.elapsed())
        .filter(|remaining| !remaining.is_zero())
    {
        Some(remaining) => probe(client, remaining).await,
        None => forward_proxy_latency_round_budget_exhausted_result(),
    }
}

fn forward_proxy_latency_round_budget_exhausted_result() -> ForwardProxyLatencyProbeTargetResult {
    ForwardProxyLatencyProbeTargetResult {
        ok: false,
        latency_ms: None,
        ip: None,
        http_status: None,
        error: Some("manual latency test round budget exhausted".to_string()),
    }
}

fn forward_proxy_latency_round_error(error: String) -> ForwardProxyLatencyRoundResults {
    let mut results = ForwardProxyLatencyRoundResults::budget_exhausted();
    results.egress_ip.error = Some(error.clone());
    results.oauth_upstream.error = Some(error.clone());
    results.codex_responses.error = Some(error);
    results
}

fn forward_proxy_latency_round_progress(
    endpoint: &ForwardProxyEndpoint,
    accumulator: &ForwardProxyLatencyAccumulator,
    results: &ForwardProxyLatencyRoundResults,
    round: usize,
    single_started: Instant,
    is_single_node_test: bool,
) -> ForwardProxyLatencyTestNodeProgress {
    let done = round >= forward_proxy_manual_latency_round_count()
        || (is_single_node_test
            && timeout_budget_exhausted(
                forward_proxy_manual_latency_single_timeout(),
                single_started.elapsed(),
            ));
    let all_targets_ok = accumulator.all_targets_ok();
    let failed_targets = accumulator.failed_targets();
    let (display_egress_ip, display_oauth_upstream, display_codex_responses) =
        accumulated_forward_proxy_latency_target_results(
            accumulator,
            &results.egress_ip,
            &results.oauth_upstream,
            &results.codex_responses,
        );
    let timed_out = done
        && [
            &results.egress_ip,
            &results.oauth_upstream,
            &results.codex_responses,
        ]
        .into_iter()
        .any(forward_proxy_latency_target_timed_out);
    let message = if !failed_targets.is_empty() {
        format!("failed targets: {}", failed_targets.join(", "))
    } else if let Some(avg) = accumulator.average_latency_ms() {
        format!(
            "{avg} ms from {}/{} successful samples",
            accumulator.success_count,
            accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT
        )
    } else if done {
        "timeout".to_string()
    } else {
        "waiting for first successful sample".to_string()
    };

    ForwardProxyLatencyTestNodeProgress {
        key: endpoint.key.clone(),
        display_name: endpoint.display_name.clone(),
        round,
        total_rounds: forward_proxy_manual_latency_round_count(),
        completed_rounds: accumulator.completed_rounds,
        success_count: accumulator.success_count,
        attempt_count: accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT,
        average_latency_ms: accumulator.average_latency_ms(),
        egress_ip: display_egress_ip,
        oauth_upstream: display_oauth_upstream,
        codex_responses: display_codex_responses,
        all_targets_ok,
        failed_targets,
        done,
        timed_out,
        message,
    }
}

pub(crate) fn forward_proxy_latency_timeout_progress(
    endpoint: &ForwardProxyEndpoint,
    accumulator: &ForwardProxyLatencyAccumulator,
) -> ForwardProxyLatencyTestNodeProgress {
    let timeout_result = ForwardProxyLatencyProbeTargetResult {
        ok: false,
        latency_ms: None,
        ip: None,
        http_status: None,
        error: Some("manual latency test budget exhausted".to_string()),
    };
    let (egress_ip, oauth_upstream, codex_responses) = if accumulator.completed_rounds == 0 {
        (
            timeout_result.clone(),
            timeout_result.clone(),
            timeout_result.clone(),
        )
    } else {
        (
            accumulator
                .last_egress_ip
                .clone()
                .unwrap_or_else(|| timeout_result.clone()),
            accumulator
                .last_oauth_upstream
                .clone()
                .unwrap_or_else(|| timeout_result.clone()),
            accumulator
                .last_codex_responses
                .clone()
                .unwrap_or_else(|| timeout_result.clone()),
        )
    };
    let failed_targets = if accumulator.completed_rounds == 0 {
        failed_forward_proxy_latency_targets(&timeout_result, &timeout_result, &timeout_result)
    } else {
        accumulator.failed_targets()
    };
    let all_targets_ok = accumulator.all_targets_ok();

    ForwardProxyLatencyTestNodeProgress {
        key: endpoint.key.clone(),
        display_name: endpoint.display_name.clone(),
        round: accumulator
            .completed_rounds
            .min(forward_proxy_manual_latency_round_count()),
        total_rounds: forward_proxy_manual_latency_round_count(),
        completed_rounds: accumulator.completed_rounds,
        success_count: accumulator.success_count,
        attempt_count: accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT,
        average_latency_ms: accumulator.average_latency_ms(),
        egress_ip,
        oauth_upstream,
        codex_responses,
        all_targets_ok,
        failed_targets: failed_targets.clone(),
        done: true,
        timed_out: !all_targets_ok,
        message: if !failed_targets.is_empty() {
            format!("failed targets: {}", failed_targets.join(", "))
        } else {
            accumulator
                .average_latency_ms()
                .map(|avg| {
                    format!(
                        "{avg} ms from {}/{} successful samples",
                        accumulator.success_count,
                        accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT
                    )
                })
                .unwrap_or_else(|| "timeout".to_string())
        },
    }
}

pub(crate) fn stream_forward_proxy_latency_tests(
    state: Arc<AppState>,
    endpoints: Vec<ForwardProxyEndpoint>,
    is_single_node_test: bool,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(
        (
            state,
            endpoints,
            Vec::<ForwardProxyLatencyAccumulator>::new(),
            1usize,
            0usize,
            Vec::<Instant>::new(),
        ),
        move |(state, endpoints, mut accumulators, mut round, mut index, mut started_at)| async move {
            if accumulators.is_empty() {
                accumulators = vec![ForwardProxyLatencyAccumulator::default(); endpoints.len()];
                started_at = (0..endpoints.len()).map(|_| Instant::now()).collect();
            }
            if round > forward_proxy_manual_latency_round_count() || endpoints.is_empty() {
                return None;
            }
            if index < endpoints.len()
                && is_single_node_test
                && timeout_budget_exhausted(
                    forward_proxy_manual_latency_single_timeout(),
                    started_at[index].elapsed(),
                )
            {
                let endpoint = endpoints[index].clone();
                let progress =
                    forward_proxy_latency_timeout_progress(&endpoint, &accumulators[index]);
                index += 1;
                round = forward_proxy_manual_latency_round_count() + 1;
                let payload = ForwardProxyLatencyTestStreamEvent {
                    kind: "completed",
                    node: progress,
                };
                let event = forward_proxy_latency_test_event("completed", &payload).map(Ok);
                return event.map(|event| {
                    (
                        event,
                        (state, endpoints, accumulators, round, index, started_at),
                    )
                });
            }
            if index >= endpoints.len() {
                round += 1;
                index = 0;
                if round > forward_proxy_manual_latency_round_count() {
                    return None;
                }
            }
            let endpoint = endpoints[index].clone();
            let progress = run_forward_proxy_latency_test_round(
                state.clone(),
                &endpoint,
                &mut accumulators[index],
                round,
                started_at[index],
                is_single_node_test,
            )
            .await;
            let event_name = if progress.done {
                "completed"
            } else {
                "progress"
            };
            let payload = ForwardProxyLatencyTestStreamEvent {
                kind: event_name,
                node: progress,
            };
            index += 1;
            let event = forward_proxy_latency_test_event(event_name, &payload).map(Ok);
            event.map(|event| {
                (
                    event,
                    (state, endpoints, accumulators, round, index, started_at),
                )
            })
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

pub(crate) async fn stream_forward_proxy_node_latency_test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(proxy_key): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let endpoints =
        load_forward_proxy_endpoints_for_latency_test(state.as_ref(), &[proxy_key]).await?;
    Ok(stream_forward_proxy_latency_tests(state, endpoints, true))
}

pub(crate) async fn stream_forward_proxy_nodes_latency_test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let proxy_keys = parse_forward_proxy_nodes_latency_test_keys(uri.query().unwrap_or_default());
    let endpoints =
        load_forward_proxy_endpoints_for_latency_test(state.as_ref(), &proxy_keys).await?;
    Ok(stream_forward_proxy_latency_tests(state, endpoints, false))
}

pub(crate) async fn post_forward_proxy_candidate_validation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ForwardProxyCandidateValidationRequest>,
) -> Result<Json<ForwardProxyCandidateValidationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let result = match payload.kind {
        ForwardProxyValidationKind::ProxyUrl => {
            validate_single_forward_proxy_candidate(state.as_ref(), payload.value).await
        }
        ForwardProxyValidationKind::SubscriptionUrl => {
            validate_subscription_candidate(state.clone(), payload.value).await
        }
    };

    let response = match result {
        Ok(response) => response,
        Err(err) => {
            warn!(error = %err, "forward proxy candidate validation failed");
            ForwardProxyCandidateValidationResponse::failed(err.to_string())
        }
    };

    Ok(Json(response))
}

pub(crate) async fn validate_single_forward_proxy_candidate(
    state: &AppState,
    value: String,
) -> Result<ForwardProxyCandidateValidationResponse> {
    let parsed = parse_forward_proxy_entry(value.trim())
        .ok_or_else(|| anyhow!("unsupported proxy url or unsupported scheme"))?;
    let endpoint = ForwardProxyEndpoint {
        key: format!(
            "__validate_proxy__{:016x}",
            stable_hash_u64(&parsed.normalized)
        ),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: parsed.display_name,
        protocol: parsed.protocol,
        endpoint_url: parsed.endpoint_url,
        raw_url: Some(parsed.normalized.clone()),
    };
    let latency_ms = probe_forward_proxy_endpoint(
        state,
        &endpoint,
        forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl),
        None,
    )
    .await?
    .expect("validation probes should not be cancelled without a shutdown token");
    Ok(ForwardProxyCandidateValidationResponse::success(
        "proxy validation succeeded",
        Some(parsed.normalized),
        Some(1),
        Some(latency_ms),
    ))
}

pub(crate) async fn validate_subscription_candidate(
    state: Arc<AppState>,
    value: String,
) -> Result<ForwardProxyCandidateValidationResponse> {
    let validation_timeout =
        forward_proxy_validation_timeout(ForwardProxyValidationKind::SubscriptionUrl);
    let validation_started = Instant::now();
    let normalized_subscription = normalize_subscription_entries(vec![value])
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("subscription url must be a valid http/https url"))?;
    let urls = fetch_subscription_proxy_urls_with_validation_budget(
        &state.http_clients.shared,
        &normalized_subscription,
        validation_timeout,
        validation_started,
    )
    .await
    .context("failed to fetch or decode subscription payload")?;
    if urls.is_empty() {
        bail!("subscription resolved zero proxy entries");
    }
    let endpoints = normalize_proxy_endpoints_from_urls(&urls, FORWARD_PROXY_SOURCE_SUBSCRIPTION);
    if endpoints.is_empty() {
        bail!("subscription contains no supported proxy entries");
    }

    let discovered_nodes = endpoints.len();
    let latency_ms = validate_subscription_endpoints_concurrently(
        state,
        endpoints,
        validation_timeout,
        validation_started,
    )
    .await?;

    Ok(ForwardProxyCandidateValidationResponse::success(
        "subscription validation succeeded",
        Some(normalized_subscription),
        Some(discovered_nodes),
        Some(latency_ms),
    ))
}

pub(crate) async fn validate_subscription_endpoints_concurrently(
    state: Arc<AppState>,
    endpoints: Vec<ForwardProxyEndpoint>,
    validation_timeout: Duration,
    validation_started: Instant,
) -> Result<f64> {
    let endpoint_count = endpoints.len();
    let concurrency = FORWARD_PROXY_SUBSCRIPTION_PROBE_CONCURRENCY.max(1);
    let attempts = FORWARD_PROXY_SUBSCRIPTION_PROBE_ATTEMPTS.max(1);
    let attempt_timeout =
        Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_PROBE_ATTEMPT_TIMEOUT_SECS.max(1));
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = ProbeCancellationGuard(cancellation.clone());
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let (tx, mut rx) = mpsc::channel::<Result<f64, String>>(endpoint_count.max(1));

    for endpoint in endpoints {
        let state = state.clone();
        let semaphore = semaphore.clone();
        let cancellation = cancellation.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let _permit = match semaphore.acquire_owned().await {
                Ok(permit) => permit,
                Err(err) => {
                    let _ = tx
                        .send(Err(format!(
                            "subscription validation concurrency limiter closed: {err}"
                        )))
                        .await;
                    return;
                }
            };
            if cancellation.is_cancelled() {
                return;
            }
            let result = probe_subscription_endpoint_with_retries(
                state.as_ref(),
                &endpoint,
                attempts,
                attempt_timeout,
                validation_timeout,
                validation_started,
                &cancellation,
            )
            .await;
            match result {
                Ok(latency_ms) => {
                    cancellation.cancel();
                    let _ = tx.send(Ok(latency_ms)).await;
                }
                Err(err) if cancellation.is_cancelled() => {
                    let _ = tx.send(Err(format!("{err:#}"))).await;
                }
                Err(err) => {
                    let _ = tx.send(Err(format!("{err:#}"))).await;
                }
            }
        });
    }
    drop(tx);

    let context = SubscriptionValidationContext {
        cancellation: cancellation.clone(),
        endpoint_count,
        concurrency,
        attempts,
        attempt_timeout,
        validation_timeout,
        validation_started,
    };
    await_subscription_validation_results(&context, &mut rx).await
}

struct SubscriptionValidationContext {
    cancellation: CancellationToken,
    endpoint_count: usize,
    concurrency: usize,
    attempts: usize,
    attempt_timeout: Duration,
    validation_timeout: Duration,
    validation_started: Instant,
}

async fn await_subscription_validation_results(
    context: &SubscriptionValidationContext,
    rx: &mut mpsc::Receiver<Result<f64, String>>,
) -> Result<f64> {
    let mut completed = 0usize;
    let mut last_error = None;
    while completed < context.endpoint_count {
        let Some(remaining_timeout) = remaining_timeout_budget(
            context.validation_timeout,
            context.validation_started.elapsed(),
        ) else {
            context.cancellation.cancel();
            return Err(timeout_error_for_duration(context.validation_timeout));
        };
        let result = match timeout(remaining_timeout, rx.recv()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_) => {
                context.cancellation.cancel();
                return Err(timeout_error_for_duration(context.validation_timeout));
            }
        };
        completed += 1;
        match result {
            Ok(latency_ms) => {
                context.cancellation.cancel();
                return Ok(latency_ms);
            }
            Err(err) => last_error = Some(err),
        }
    }
    let mut message = format!(
        "subscription validation scanned {} proxy entries with concurrency {}, {} attempts per entry, and {}s per attempt; no entry passed validation",
        context.endpoint_count,
        context.concurrency,
        context.attempts,
        timeout_seconds_for_message(context.attempt_timeout)
    );
    if let Some(err) = last_error {
        message.push_str(&format!("; last error: {err}"));
    }
    bail!(message)
}

pub(crate) struct ProbeCancellationGuard(CancellationToken);

impl Drop for ProbeCancellationGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
