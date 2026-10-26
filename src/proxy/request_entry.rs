use super::*;

mod attempt_lifecycle;
mod image_tool_rewrite;
mod invocation_identity;
mod replay_body;
mod response_capture;
mod responses_overload;
mod semantic_projection;

pub(crate) use attempt_lifecycle::*;
pub(crate) use image_tool_rewrite::*;
pub(crate) use invocation_identity::PoolInvocationCleanupGuard;
pub(crate) use replay_body::*;
pub(crate) use response_capture::*;
pub(crate) use responses_overload::*;
pub(crate) use semantic_projection::*;

const WEBSOCKET_PROXY_REMOVED_MESSAGE: &str = "WebSocket proxy support has been removed";
const WEBSOCKET_PROXY_REMOVED_CODE: &str = "websocket_proxy_removed";
const WEBSOCKET_PROXY_REMOVED_BODY: &str =
    r#"{"error":"WebSocket proxy support has been removed","code":"websocket_proxy_removed"}"#;

pub(crate) async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if state.startup_ready.load(Ordering::Acquire) {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "starting")
    }
}

#[cfg(test)]
pub(crate) async fn proxy_openai_v1(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    Box::pin(proxy_openai_v1_common(
        state,
        original_uri,
        method,
        headers,
        body,
        None,
        None,
    ))
    .await
}

pub(crate) async fn proxy_openai_v1_with_connect_info(
    State(state): State<Arc<AppState>>,
    connect_info: Result<ConnectInfo<SocketAddr>, axum::extract::rejection::ExtensionRejection>,
    downstream_transport: Option<Extension<DownstreamTransportObserver>>,
    http2_protocol: Option<Extension<hyper::ext::Protocol>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    if !state.startup_ready.load(Ordering::Acquire) {
        return startup_not_ready_response();
    }
    if is_websocket_upgrade_request(&headers, http2_protocol.as_ref()) {
        info!(
            method = %method,
            uri_path = %original_uri.path(),
            websocket_proxy_rejected = true,
            "downstream WebSocket proxy request rejected"
        );
        return build_proxy_error_response(
            ProxyErrorResponse {
                status: StatusCode::NOT_IMPLEMENTED,
                message: WEBSOCKET_PROXY_REMOVED_MESSAGE.to_string(),
                cvm_id: None,
                retry_after_secs: None,
                code: Some(WEBSOCKET_PROXY_REMOVED_CODE.to_string()),
                blocked_binding: None,
            },
            "websocket-proxy-removed",
        );
    }
    Box::pin(proxy_openai_v1_common(
        state,
        original_uri,
        method,
        headers,
        body,
        connect_info.as_ref().ok().map(|info| info.0.ip()),
        downstream_transport.map(|Extension(observer)| observer),
    ))
    .await
}

fn is_websocket_upgrade_request(
    headers: &HeaderMap,
    http2_protocol: Option<&Extension<hyper::ext::Protocol>>,
) -> bool {
    headers
        .get_all(header::UPGRADE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("websocket"))
        })
        || http2_protocol
            .is_some_and(|Extension(protocol)| protocol.as_str().eq_ignore_ascii_case("websocket"))
}

fn proxy_uri_path_for_log(uri: &Uri) -> &str {
    uri.path()
}

pub(crate) async fn proxy_openai_v1_common(
    state: Arc<AppState>,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
    downstream_transport: Option<DownstreamTransportObserver>,
) -> Response {
    if !state.startup_ready.load(Ordering::Acquire) {
        return startup_not_ready_response();
    }
    if method == Method::GET
        && is_models_list_path(original_uri.path())
        && !request_matches_pool_route(state.as_ref(), &headers)
            .await
            .unwrap_or(false)
    {
        // Public discovery exposes only local presets; upstream access remains authenticated.
        let settings = state.proxy_model_settings.read().await;
        return Json(build_preset_models_payload(&settings.enabled_preset_models)).into_response();
    }

    let diagnostic_auth_route = crate::observability::diagnostics::phase(
        crate::observability::diagnostics::Phase::AuthRoute,
    );
    let proxy_request_id = next_proxy_request_id();
    let started_at = Instant::now();
    let request_content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    let request_may_have_body = request_may_have_body(&method, &headers);
    let method_for_log = method.clone();
    let uri_for_log = original_uri.clone();
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);

    info!(
        proxy_request_id,
        method = %method_for_log,
        uri_path = %proxy_uri_path_for_log(&uri_for_log),
        proxy_request_started = true,
        has_body = request_may_have_body,
        content_length = ?request_content_length,
        peer_ip = ?peer_ip,
        "openai proxy request started"
    );

    let target_url = match build_proxy_upstream_url(
        &state.config.openai_upstream_base_url,
        &original_uri,
    ) {
        Ok(url) => url,
        Err(err) => {
            let status = if err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
                || err.to_string().contains(PROXY_INVALID_REQUEST_TARGET)
                || err
                    .to_string()
                    .contains("failed to parse proxy upstream url")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            let invoke_id = match allocate_proxy_invoke_id_without_active_lease(
                &state,
                header_prompt_cache_key.as_deref(),
            )
            .await
            {
                Ok(invoke_id) => invoke_id,
                Err(error) => {
                    warn!(
                        proxy_request_id,
                        error = %error,
                        "failed to allocate proxy invoke id after upstream URL validation failure"
                    );
                    return build_proxy_invoke_id_allocation_error_response(error);
                }
            };
            return build_proxy_error_response(
                ProxyErrorResponse {
                    status,
                    message: format!("failed to build upstream url: {err}"),
                    cvm_id: None,
                    retry_after_secs: None,
                    code: None,
                    blocked_binding: None,
                },
                &invoke_id,
            );
        }
    };

    let proxy_request_permit = Some(
        acquire_proxy_request_concurrency_permit(
            state.as_ref(),
            proxy_request_id,
            &method_for_log,
            &uri_for_log,
        )
        .await,
    );
    let capture_target = capture_target_for_request(original_uri.path(), &method);
    let transport_request_observer = downstream_transport
        .as_ref()
        .map(DownstreamTransportObserver::begin_request);
    let downstream_request_observer = capture_target
        .is_some()
        .then_some(transport_request_observer)
        .flatten();

    if extract_bearer_token(&headers).is_none() {
        let err = ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        };
        let invoke_id =
            match allocate_proxy_invoke_id(&state, header_prompt_cache_key.as_deref()).await {
                Ok(invoke_id) => invoke_id,
                Err(error) => {
                    warn!(
                        proxy_request_id,
                        error = %error,
                        "failed to allocate proxy invoke id for missing bearer token"
                    );
                    return build_proxy_invoke_id_allocation_error_response(error);
                }
            };
        if let Some(target) = capture_target {
            let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let requester_ip = extract_requester_ip(&headers, peer_ip);
            let header_sticky_key = extract_sticky_key_from_headers(&headers);
            emit_admitted_proxy_capture_runtime_snapshot(
                state.as_ref(),
                &invoke_id,
                &occurred_at,
                target,
                requester_ip.as_deref(),
                header_sticky_key.as_deref(),
                header_prompt_cache_key.as_deref(),
            )
            .await;
            terminalize_proxy_runtime_snapshot_with_error(
                state.as_ref(),
                &invoke_id,
                &occurred_at,
                err.status,
                PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                &err.message,
                "missing_bearer_token",
            );
            schedule_dashboard_activity_live_snapshot(state.as_ref());
        }
        return build_proxy_error_response(err, &invoke_id);
    }

    let route_context_started = Instant::now();
    let runtime_timeouts = match resolve_proxy_route_context_for_request(
        state.as_ref(),
        proxy_request_id,
        &method_for_log,
        &uri_for_log,
        &headers,
    )
    .await
    {
        Ok(route_context) => route_context,
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method_for_log,
                uri_path = %proxy_uri_path_for_log(&uri_for_log),
                status = %err.status,
                error = %err.message,
                route_context_elapsed = route_context_started.elapsed().as_millis() as u64,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed during route validation"
            );
            let invoke_id =
                match allocate_proxy_invoke_id(&state, header_prompt_cache_key.as_deref()).await {
                    Ok(invoke_id) => invoke_id,
                    Err(error) => {
                        warn!(
                            proxy_request_id,
                            error = %error,
                            "failed to allocate proxy invoke id for route validation failure"
                        );
                        return build_proxy_invoke_id_allocation_error_response(error);
                    }
                };
            if let Some(target) = capture_target {
                let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
                let requester_ip = extract_requester_ip(&headers, peer_ip);
                let header_sticky_key = extract_sticky_key_from_headers(&headers);
                emit_admitted_proxy_capture_runtime_snapshot(
                    state.as_ref(),
                    &invoke_id,
                    &occurred_at,
                    target,
                    requester_ip.as_deref(),
                    header_sticky_key.as_deref(),
                    header_prompt_cache_key.as_deref(),
                )
                .await;
                terminalize_proxy_runtime_snapshot_with_error(
                    state.as_ref(),
                    &invoke_id,
                    &occurred_at,
                    err.status,
                    PROXY_FAILURE_POOL_ROUTING_BLOCKED,
                    &err.message,
                    "route_validation_failed",
                );
                schedule_dashboard_activity_live_snapshot(state.as_ref());
            }
            return build_proxy_error_response(err, &invoke_id);
        }
    };
    debug!(
        proxy_request_id,
        route_context_elapsed = route_context_started.elapsed().as_millis() as u64,
        "proxy route context resolved"
    );
    if let Some(guard) = diagnostic_auth_route {
        guard.complete();
    }
    let pool_route_active = true;
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);

    match Box::pin(proxy_openai_v1_inner(
        state.clone(),
        proxy_request_id,
        original_uri,
        method,
        headers,
        body,
        target_url,
        peer_ip,
        pool_route_active,
        runtime_timeouts,
        proxy_request_permit,
        downstream_request_observer,
        started_at,
    ))
    .await
    {
        Ok(response) => {
            let status = response.status();
            info!(
                proxy_request_id,
                method = %method_for_log,
                uri_path = %proxy_uri_path_for_log(&uri_for_log),
                status = %status,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy response headers ready"
            );
            response
        }
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method_for_log,
                uri_path = %proxy_uri_path_for_log(&uri_for_log),
                status = %err.status,
                error = %err.message,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed"
            );
            let invoke_id = match err.cvm_id.clone() {
                Some(invoke_id) => invoke_id,
                None => match allocate_proxy_invoke_id(&state, header_prompt_cache_key.as_deref())
                    .await
                {
                    Ok(invoke_id) => invoke_id,
                    Err(error) => {
                        warn!(
                            proxy_request_id,
                            prompt_cache_key_fingerprint = header_prompt_cache_key
                                .as_deref()
                                .map(prompt_cache_key_fingerprint),
                            error = %error,
                            "failed to allocate proxy invoke id for proxy error response"
                        );
                        return build_proxy_invoke_id_allocation_error_response(error);
                    }
                },
            };
            build_proxy_error_response(err, &invoke_id)
        }
    }
}

fn startup_not_ready_response() -> Response {
    build_proxy_error_response_without_invoke_id(ProxyErrorResponse {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "service is starting".to_string(),
        cvm_id: None,
        retry_after_secs: None,
        code: Some("startup_in_progress".to_string()),
        blocked_binding: None,
    })
}

#[derive(Debug)]
pub(crate) struct ProxyRequestConcurrencyPermit {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for ProxyRequestConcurrencyPermit {
    fn drop(&mut self) {
        let _ = self
            .in_flight
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_sub(1))
            });
    }
}

pub(crate) async fn acquire_proxy_request_concurrency_permit(
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
) -> ProxyRequestConcurrencyPermit {
    let in_flight = state
        .proxy_request_in_flight
        .fetch_add(1, Ordering::AcqRel)
        .saturating_add(1);
    info!(
        proxy_request_id,
        method = %method,
        uri_path = %proxy_uri_path_for_log(original_uri),
        in_flight,
        proxy_request_admitted_observed = true,
        max_proxy_in_flight_observed = in_flight,
        "proxy request admitted"
    );

    ProxyRequestConcurrencyPermit {
        in_flight: state.proxy_request_in_flight.clone(),
    }
}

pub(crate) async fn take_or_acquire_proxy_request_concurrency_permit(
    permit: &mut Option<ProxyRequestConcurrencyPermit>,
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
) -> ProxyRequestConcurrencyPermit {
    match permit.take() {
        Some(permit) => permit,
        None => {
            acquire_proxy_request_concurrency_permit(state, proxy_request_id, method, original_uri)
                .await
        }
    }
}

pub(crate) async fn resolve_proxy_route_context_for_request(
    state: &AppState,
    proxy_request_id: u64,
    method: &Method,
    original_uri: &Uri,
    headers: &HeaderMap,
) -> Result<PoolRoutingTimeoutSettingsResolved, ProxyErrorResponse> {
    let pool_route_active = match request_matches_pool_route(state, headers).await {
        Ok(active) => active,
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method,
                uri_path = %proxy_uri_path_for_log(original_uri),
                error = %err,
                "failed to resolve pool route"
            );
            return Err(ProxyErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: format!("failed to resolve pool routing settings: {err}"),
                cvm_id: None,
                retry_after_secs: None,
                code: None,
                blocked_binding: None,
            });
        }
    };

    if !pool_route_active {
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE.to_string(),
            cvm_id: None,
            retry_after_secs: None,
            code: None,
            blocked_binding: None,
        });
    }

    match resolve_proxy_request_timeouts(state, true).await {
        Ok(timeouts) => Ok(timeouts),
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method,
                uri_path = %proxy_uri_path_for_log(original_uri),
                error = %err,
                "failed to resolve pool routing timeouts"
            );
            Err(ProxyErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: format!("failed to resolve pool routing timeouts: {err}"),
                cvm_id: None,
                retry_after_secs: None,
                code: None,
                blocked_binding: None,
            })
        }
    }
}

pub(crate) async fn resolve_proxy_request_timeouts(
    state: &AppState,
    pool_route_active: bool,
) -> Result<PoolRoutingTimeoutSettingsResolved> {
    if pool_route_active {
        Ok(load_pool_routing_runtime_cache(state).await?.timeouts)
    } else {
        Ok(pool_routing_timeouts_from_config(&state.config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_uri_path_for_log_excludes_query_parameters() {
        let uri: Uri = "/v1/responses?api_key=secret-value"
            .parse()
            .expect("valid request URI");

        assert_eq!(proxy_uri_path_for_log(&uri), "/v1/responses");
    }
}
