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
