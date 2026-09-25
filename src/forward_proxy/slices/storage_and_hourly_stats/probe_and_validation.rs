use super::*;

pub(crate) fn parse_forward_proxy_nodes_latency_test_keys(raw_query: &str) -> Vec<String> {
    url::form_urlencoded::parse(raw_query.as_bytes())
        .filter_map(|(key, value)| (key == "key").then(|| value.into_owned()))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLatencyProbeTargetResult {
    pub(crate) ok: bool,
    pub(crate) latency_ms: Option<f64>,
    pub(crate) ip: Option<String>,
    pub(crate) http_status: Option<u16>,
    pub(crate) error: Option<String>,
}

impl ForwardProxyLatencyProbeTargetResult {
    fn success_latency(&self) -> Option<f64> {
        self.ok.then_some(self.latency_ms).flatten()
    }
}

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

    let mut completed = 0usize;
    let mut last_error: Option<String> = None;
    loop {
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            cancellation.cancel();
            return Err(timeout_error_for_duration(validation_timeout));
        };
        if remaining_timeout.is_zero() {
            cancellation.cancel();
            return Err(timeout_error_for_duration(validation_timeout));
        }

        let result = match timeout(remaining_timeout, rx.recv()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_) => {
                cancellation.cancel();
                return Err(timeout_error_for_duration(validation_timeout));
            }
        };

        completed += 1;
        match result {
            Ok(latency_ms) => {
                cancellation.cancel();
                return Ok(latency_ms);
            }
            Err(err) => last_error = Some(err),
        }
        if completed >= endpoint_count {
            break;
        }
    }

    let mut message = format!(
        "subscription validation scanned {endpoint_count} proxy entries with concurrency {concurrency}, {attempts} attempts per entry, and {}s per attempt; no entry passed validation",
        timeout_seconds_for_message(attempt_timeout)
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

pub(crate) async fn probe_subscription_endpoint_with_retries(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    attempts: usize,
    attempt_timeout: Duration,
    validation_timeout: Duration,
    validation_started: Instant,
    cancellation: &CancellationToken,
) -> Result<f64> {
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=attempts {
        if cancellation.is_cancelled() {
            return Err(shutdown_cancelled_forward_proxy_probe());
        }
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            return Err(timeout_error_for_duration(validation_timeout));
        };
        if remaining_timeout.is_zero() {
            return Err(timeout_error_for_duration(validation_timeout));
        }

        let probe_result = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(shutdown_cancelled_forward_proxy_probe());
            }
            _ = tokio::time::sleep(remaining_timeout) => {
                return Err(timeout_error_for_duration(validation_timeout));
            }
            result = probe_forward_proxy_endpoint(state, endpoint, attempt_timeout, Some(cancellation)) => {
                result
            }
        };

        match probe_result {
            Ok(Some(latency_ms)) => return Ok(latency_ms),
            Ok(None) => return Err(shutdown_cancelled_forward_proxy_probe()),
            Err(err) => {
                last_error = Some(err.context(format!(
                    "attempt {attempt}/{attempts} failed for {}",
                    endpoint.display_name
                )));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("subscription proxy probe did not run")))
}

pub(crate) fn shutdown_cancelled_forward_proxy_probe() -> anyhow::Error {
    anyhow!("forward proxy probe cancelled because shutdown is in progress")
}

pub(crate) async fn probe_forward_proxy_endpoint(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    validation_timeout: Duration,
    shutdown: Option<&CancellationToken>,
) -> Result<Option<f64>> {
    if shutdown.is_some_and(CancellationToken::is_cancelled) {
        return Ok(None);
    }

    let probe_target = state
        .config
        .openai_upstream_base_url
        .join("v1/models")
        .context("failed to build validation probe target")?;
    let started = Instant::now();
    let (endpoint_url, temporary_xray_key) = match resolve_forward_proxy_probe_endpoint_url(
        state,
        endpoint,
        validation_timeout,
        shutdown,
    )
    .await
    {
        Ok(result) => result,
        Err(_err) if shutdown.is_some_and(CancellationToken::is_cancelled) => {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };

    let probe_result = async {
        let send_timeout = remaining_timeout_budget(validation_timeout, started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| timeout_error_for_duration(validation_timeout))?;
        let client = state
            .http_clients
            .client_for_forward_proxy(endpoint_url.as_ref())?;
        let response = match shutdown {
            Some(shutdown) => {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        return Ok(None);
                    }
                    response = timeout(send_timeout, client.get(probe_target).send()) => {
                        response
                            .map_err(|_| timeout_error_for_duration(validation_timeout))?
                            .context("validation request failed")?
                    }
                }
            }
            None => timeout(send_timeout, client.get(probe_target).send())
                .await
                .map_err(|_| timeout_error_for_duration(validation_timeout))?
                .context("validation request failed")?,
        };
        let status = response.status();
        // Validation only needs to prove the route is reachable; auth/404 still count as reachable.
        if !is_validation_probe_reachable_status(status) {
            bail!("validation probe returned status {}", status);
        }
        if shutdown.is_some_and(CancellationToken::is_cancelled) {
            return Ok(None);
        }
        Ok(Some(elapsed_ms(started)))
    }
    .await;

    if let Some(temp_key) = temporary_xray_key {
        let mut supervisor = state.xray_supervisor.lock().await;
        supervisor.remove_instance(&temp_key).await;
    }

    probe_result
}

pub(crate) fn is_validation_probe_reachable_status(status: StatusCode) -> bool {
    status.is_success()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status == StatusCode::NOT_FOUND
}

pub(crate) fn forward_proxy_validation_timeout(kind: ForwardProxyValidationKind) -> Duration {
    match kind {
        ForwardProxyValidationKind::ProxyUrl => {
            Duration::from_secs(FORWARD_PROXY_VALIDATION_TIMEOUT_SECS)
        }
        ForwardProxyValidationKind::SubscriptionUrl => {
            Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS)
        }
    }
}

pub(crate) fn remaining_timeout_budget(
    total_timeout: Duration,
    elapsed: Duration,
) -> Option<Duration> {
    total_timeout.checked_sub(elapsed)
}

pub(crate) fn timeout_budget_exhausted(total_timeout: Duration, elapsed: Duration) -> bool {
    match remaining_timeout_budget(total_timeout, elapsed) {
        Some(remaining) => remaining.is_zero(),
        None => true,
    }
}

pub(crate) fn timeout_error_for_duration(timeout: Duration) -> anyhow::Error {
    anyhow!(
        "validation request timed out after {}s",
        timeout_seconds_for_message(timeout)
    )
}

pub(crate) fn timeout_seconds_for_message(timeout: Duration) -> u64 {
    let secs = timeout.as_secs();
    if timeout.subsec_nanos() > 0 {
        secs.saturating_add(1).max(1)
    } else {
        secs.max(1)
    }
}

pub(crate) async fn resolve_forward_proxy_probe_endpoint_url(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    validation_timeout: Duration,
    shutdown: Option<&CancellationToken>,
) -> Result<(Option<Url>, Option<String>)> {
    if shutdown.is_some_and(CancellationToken::is_cancelled) {
        return Err(shutdown_cancelled_forward_proxy_probe());
    }
    if !endpoint.requires_xray() {
        return Ok((endpoint.endpoint_url.clone(), None));
    }
    let raw_url = endpoint
        .raw_url
        .as_deref()
        .ok_or_else(|| anyhow!("xray proxy validation requires raw proxy url"))?;
    let temporary_key = format!(
        "__validate_xray__{:016x}_{}",
        stable_hash_u64(raw_url),
        Utc::now().timestamp_millis()
    );
    let probe_endpoint = ForwardProxyEndpoint {
        key: temporary_key.clone(),
        source: endpoint.source.clone(),
        display_name: endpoint.display_name.clone(),
        protocol: endpoint.protocol,
        endpoint_url: None,
        raw_url: Some(raw_url.to_string()),
    };
    let validation_shutdown = shutdown.cloned().unwrap_or_else(CancellationToken::new);
    let route_url = {
        let mut supervisor = state.xray_supervisor.lock().await;
        supervisor
            .ensure_instance_with_ready_timeout(
                &probe_endpoint,
                validation_timeout,
                &validation_shutdown,
            )
            .await?
    };
    Ok((Some(route_url), Some(temporary_key)))
}

pub(crate) fn snapshot_active_forward_proxy_endpoints(
    manager: &ForwardProxyManager,
) -> Vec<ForwardProxyEndpoint> {
    manager
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.protocol != ForwardProxyProtocol::Direct)
        .filter(|endpoint| endpoint.endpoint_url.is_some() || endpoint.requires_xray())
        .cloned()
        .collect()
}

pub(crate) fn compute_added_forward_proxy_endpoints(
    before: &[ForwardProxyEndpoint],
    after: &[ForwardProxyEndpoint],
) -> Vec<ForwardProxyEndpoint> {
    let known = before
        .iter()
        .map(|endpoint| endpoint.key.as_str())
        .collect::<HashSet<_>>();
    after
        .iter()
        .filter(|endpoint| !known.contains(endpoint.key.as_str()))
        .cloned()
        .collect()
}

pub(crate) fn snapshot_known_subscription_proxy_keys(
    manager: &ForwardProxyManager,
) -> HashSet<String> {
    manager
        .runtime
        .values()
        .filter(|entry| entry.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
        .map(|entry| entry.proxy_key.clone())
        .collect()
}

pub(crate) fn classify_bootstrap_forward_proxy_probe_failure(err: &anyhow::Error) -> &'static str {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("timed out") || message.contains("timeout") {
        return FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT;
    }
    if message.contains("validation probe returned status 5") {
        return FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX;
    }
    FORWARD_PROXY_FAILURE_SEND_ERROR
}

pub(crate) fn spawn_forward_proxy_bootstrap_probe_round(
    state: Arc<AppState>,
    added_endpoints: Vec<ForwardProxyEndpoint>,
    trigger: &'static str,
) {
    if added_endpoints.is_empty() || state.shutdown.is_cancelled() {
        return;
    }
    tokio::spawn(async move {
        let shutdown = state.shutdown.clone();
        let validation_timeout =
            forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl);
        info!(
            trigger,
            added_count = added_endpoints.len(),
            timeout_secs = validation_timeout.as_secs(),
            "forward proxy bootstrap probe round started"
        );
        for endpoint in added_endpoints {
            if shutdown.is_cancelled() {
                info!(
                    trigger,
                    "forward proxy bootstrap probe round stopped by shutdown"
                );
                break;
            }
            let selected_proxy = SelectedForwardProxy::from_endpoint(&endpoint);
            let started = Instant::now();
            let probe_result = probe_forward_proxy_endpoint(
                state.as_ref(),
                &endpoint,
                validation_timeout,
                Some(&shutdown),
            )
            .await;
            match probe_result {
                Ok(Some(latency_ms)) => {
                    if shutdown.is_cancelled() {
                        info!(
                            trigger,
                            proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                            "forward proxy bootstrap probe round stopped before recording a completed probe because shutdown is in progress"
                        );
                        break;
                    }
                    record_forward_proxy_attempt(
                        state.clone(),
                        selected_proxy,
                        true,
                        Some(latency_ms),
                        None,
                        true,
                    )
                    .await;
                }
                Ok(None) => {
                    info!(
                        trigger,
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        "forward proxy bootstrap probe round stopped by shutdown during an in-flight probe"
                    );
                    break;
                }
                Err(err) => {
                    let failure_kind = classify_bootstrap_forward_proxy_probe_failure(&err);
                    warn!(
                        trigger,
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        proxy_source = endpoint.source,
                        proxy_label = endpoint.display_name,
                        proxy_url_ref = %forward_proxy_log_ref_option(endpoint.raw_url.as_deref()),
                        failure_kind,
                        error = %err,
                        "forward proxy bootstrap probe failed"
                    );
                    record_forward_proxy_attempt(
                        state.clone(),
                        selected_proxy,
                        false,
                        Some(elapsed_ms(started)),
                        Some(failure_kind),
                        true,
                    )
                    .await;
                }
            }
        }
        info!(trigger, "forward proxy bootstrap probe round finished");
    });
}
