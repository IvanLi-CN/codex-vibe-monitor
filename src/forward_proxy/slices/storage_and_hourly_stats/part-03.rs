#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLatencyTestNodeProgress {
    pub(crate) key: String,
    pub(crate) display_name: String,
    pub(crate) round: usize,
    pub(crate) total_rounds: usize,
    pub(crate) completed_rounds: usize,
    pub(crate) success_count: usize,
    pub(crate) attempt_count: usize,
    pub(crate) average_latency_ms: Option<u64>,
    pub(crate) egress_ip: ForwardProxyLatencyProbeTargetResult,
    pub(crate) oauth_upstream: ForwardProxyLatencyProbeTargetResult,
    pub(crate) codex_responses: ForwardProxyLatencyProbeTargetResult,
    pub(crate) all_targets_ok: bool,
    pub(crate) failed_targets: Vec<&'static str>,
    pub(crate) done: bool,
    pub(crate) timed_out: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLatencyTestStreamEvent {
    kind: &'static str,
    node: ForwardProxyLatencyTestNodeProgress,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ForwardProxyLatencyAccumulator {
    pub(crate) total_latency_ms: f64,
    pub(crate) success_count: usize,
    pub(crate) completed_rounds: usize,
    pub(crate) egress_ip_failures: usize,
    pub(crate) oauth_upstream_failures: usize,
    pub(crate) codex_responses_failures: usize,
    pub(crate) last_egress_ip: Option<ForwardProxyLatencyProbeTargetResult>,
    pub(crate) last_oauth_upstream: Option<ForwardProxyLatencyProbeTargetResult>,
    pub(crate) last_codex_responses: Option<ForwardProxyLatencyProbeTargetResult>,
}

impl ForwardProxyLatencyAccumulator {
    pub(crate) fn record_round(
        &mut self,
        egress_ip: &ForwardProxyLatencyProbeTargetResult,
        oauth_upstream: &ForwardProxyLatencyProbeTargetResult,
        codex_responses: &ForwardProxyLatencyProbeTargetResult,
    ) {
        self.completed_rounds += 1;
        for latency_ms in [
            egress_ip.success_latency(),
            oauth_upstream.success_latency(),
            codex_responses.success_latency(),
        ]
        .into_iter()
        .flatten()
        {
            self.total_latency_ms += latency_ms;
            self.success_count += 1;
        }
        if !egress_ip.ok {
            self.egress_ip_failures += 1;
        }
        if !oauth_upstream.ok {
            self.oauth_upstream_failures += 1;
        }
        if !codex_responses.ok {
            self.codex_responses_failures += 1;
        }
        preserve_forward_proxy_latency_target_result(
            &mut self.last_egress_ip,
            self.egress_ip_failures,
            egress_ip,
        );
        preserve_forward_proxy_latency_target_result(
            &mut self.last_oauth_upstream,
            self.oauth_upstream_failures,
            oauth_upstream,
        );
        preserve_forward_proxy_latency_target_result(
            &mut self.last_codex_responses,
            self.codex_responses_failures,
            codex_responses,
        );
    }

    pub(crate) fn average_latency_ms(&self) -> Option<u64> {
        if self.success_count == 0 {
            return None;
        }
        Some((self.total_latency_ms / self.success_count as f64).round() as u64)
    }

    pub(crate) fn all_targets_ok(&self) -> bool {
        self.completed_rounds > 0
            && self.egress_ip_failures == 0
            && self.oauth_upstream_failures == 0
            && self.codex_responses_failures == 0
    }

    pub(crate) fn failed_targets(&self) -> Vec<&'static str> {
        let mut targets = Vec::new();
        if self.egress_ip_failures > 0 {
            targets.push(FORWARD_PROXY_LATENCY_TARGET_EGRESS_IP);
        }
        if self.oauth_upstream_failures > 0 {
            targets.push(FORWARD_PROXY_LATENCY_TARGET_OAUTH_UPSTREAM);
        }
        if self.codex_responses_failures > 0 {
            targets.push(FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES);
        }
        targets
    }
}

pub(crate) fn preserve_forward_proxy_latency_target_result(
    slot: &mut Option<ForwardProxyLatencyProbeTargetResult>,
    failure_count: usize,
    result: &ForwardProxyLatencyProbeTargetResult,
) {
    if !result.ok || failure_count == 0 || slot.is_none() {
        *slot = Some(result.clone());
    }
}

pub(crate) fn accumulated_forward_proxy_latency_target_results(
    accumulator: &ForwardProxyLatencyAccumulator,
    egress_ip: &ForwardProxyLatencyProbeTargetResult,
    oauth_upstream: &ForwardProxyLatencyProbeTargetResult,
    codex_responses: &ForwardProxyLatencyProbeTargetResult,
) -> (
    ForwardProxyLatencyProbeTargetResult,
    ForwardProxyLatencyProbeTargetResult,
    ForwardProxyLatencyProbeTargetResult,
) {
    (
        accumulator
            .last_egress_ip
            .clone()
            .unwrap_or_else(|| egress_ip.clone()),
        accumulator
            .last_oauth_upstream
            .clone()
            .unwrap_or_else(|| oauth_upstream.clone()),
        accumulator
            .last_codex_responses
            .clone()
            .unwrap_or_else(|| codex_responses.clone()),
    )
}

pub(crate) fn forward_proxy_manual_latency_round_timeout() -> Duration {
    Duration::from_secs(FORWARD_PROXY_MANUAL_LATENCY_ROUND_TIMEOUT_SECS)
}

pub(crate) fn forward_proxy_manual_latency_single_timeout() -> Duration {
    Duration::from_secs(FORWARD_PROXY_MANUAL_LATENCY_SINGLE_TIMEOUT_SECS)
}

pub(crate) fn forward_proxy_manual_latency_round_count() -> usize {
    FORWARD_PROXY_MANUAL_LATENCY_TEST_ROUNDS
}

pub(crate) fn forward_proxy_latency_breadth_first_schedule(
    node_count: usize,
    total_rounds: usize,
) -> Vec<(usize, usize)> {
    (1..=total_rounds)
        .flat_map(|round| (0..node_count).map(move |node_index| (round, node_index)))
        .collect()
}

pub(crate) fn forward_proxy_latency_test_event(
    event_name: &'static str,
    payload: &ForwardProxyLatencyTestStreamEvent,
) -> Option<Event> {
    match Event::default().event(event_name).json_data(payload) {
        Ok(event) => Some(event),
        Err(err) => {
            warn!(?err, "failed to serialize forward proxy latency test event");
            None
        }
    }
}

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
    let total_timeout = if is_single_node_test {
        forward_proxy_manual_latency_single_timeout()
    } else {
        Duration::from_secs(u64::MAX / 4)
    };
    let remaining_total = remaining_timeout_budget(total_timeout, single_started.elapsed())
        .filter(|remaining| !remaining.is_zero());
    let round_timeout = remaining_total
        .map(|remaining| remaining.min(forward_proxy_manual_latency_round_timeout()))
        .unwrap_or_else(|| Duration::from_secs(0));
    let mut egress_ip = ForwardProxyLatencyProbeTargetResult {
        ok: false,
        latency_ms: None,
        ip: None,
        http_status: None,
        error: Some("manual latency test budget exhausted".to_string()),
    };
    let mut oauth_upstream = egress_ip.clone();
    let mut codex_responses = egress_ip.clone();
    let mut round_started = Instant::now();

    if !round_timeout.is_zero() {
        match resolve_forward_proxy_probe_endpoint_url(
            state.as_ref(),
            endpoint,
            round_timeout,
            Some(&state.shutdown),
        )
        .await
        {
            Ok((endpoint_url, temporary_xray_key)) => {
                let client_result = state
                    .http_clients
                    .client_for_forward_proxy(endpoint_url.as_ref());
                match client_result {
                    Ok(client) => {
                        round_started = Instant::now();
                        egress_ip = timed_forward_proxy_egress_ip_probe(
                            state.as_ref(),
                            &selected_proxy,
                            &client,
                            round_timeout,
                        )
                        .await;
                        let remaining_round =
                            remaining_timeout_budget(round_timeout, round_started.elapsed())
                                .filter(|remaining| !remaining.is_zero());
                        oauth_upstream = match remaining_round {
                            Some(remaining) => {
                                timed_forward_proxy_oauth_upstream_probe(&client, remaining).await
                            }
                            None => ForwardProxyLatencyProbeTargetResult {
                                ok: false,
                                latency_ms: None,
                                ip: None,
                                http_status: None,
                                error: Some(
                                    "manual latency test round budget exhausted".to_string(),
                                ),
                            },
                        };
                        let remaining_round =
                            remaining_timeout_budget(round_timeout, round_started.elapsed())
                                .filter(|remaining| !remaining.is_zero());
                        codex_responses = match remaining_round {
                            Some(remaining) => {
                                timed_forward_proxy_codex_responses_probe(&client, remaining).await
                            }
                            None => ForwardProxyLatencyProbeTargetResult {
                                ok: false,
                                latency_ms: None,
                                ip: None,
                                http_status: None,
                                error: Some(
                                    "manual latency test round budget exhausted".to_string(),
                                ),
                            },
                        };
                    }
                    Err(err) => {
                        egress_ip.error = Some(err.to_string());
                        oauth_upstream.error = Some(err.to_string());
                        codex_responses.error = Some(err.to_string());
                    }
                }
                if let Some(temp_key) = temporary_xray_key {
                    let mut supervisor = state.xray_supervisor.lock().await;
                    supervisor.remove_instance(&temp_key).await;
                }
            }
            Err(err) => {
                egress_ip.error = Some(err.to_string());
                oauth_upstream.error = Some(err.to_string());
                codex_responses.error = Some(err.to_string());
            }
        }
    }

    accumulator.record_round(&egress_ip, &oauth_upstream, &codex_responses);
    let round_success = egress_ip.ok && oauth_upstream.ok && codex_responses.ok;
    let round_latency_ms = {
        let samples = [
            egress_ip.success_latency(),
            oauth_upstream.success_latency(),
            codex_responses.success_latency(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if samples.is_empty() {
            None
        } else {
            Some(samples.iter().sum::<f64>() / samples.len() as f64)
        }
    };
    record_forward_proxy_attempt(
        state,
        selected_proxy,
        round_success,
        round_latency_ms.or_else(|| Some(elapsed_ms(round_started))),
        if round_success {
            None
        } else {
            Some(FORWARD_PROXY_FAILURE_SEND_ERROR)
        },
        true,
    )
    .await;
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
            &egress_ip,
            &oauth_upstream,
            &codex_responses,
        );
    let timed_out = done
        && [&egress_ip, &oauth_upstream, &codex_responses]
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
