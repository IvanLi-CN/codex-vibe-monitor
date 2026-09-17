fn begin_downstream_request_observer(
    downstream_transport: Option<&DownstreamTransportObserver>,
    capture_target: Option<ProxyCaptureTarget>,
) -> Option<DownstreamRequestObserver> {
    let transport_request_observer =
        downstream_transport.map(DownstreamTransportObserver::begin_request);
    capture_target
        .is_some()
        .then_some(transport_request_observer)
        .flatten()
}

fn clone_and_log_proxy_request(
    proxy_request_id: u64,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
) -> (Method, Uri) {
    let method_for_log = method.clone();
    let uri_for_log = uri.clone();
    log_proxy_request_started(
        proxy_request_id,
        &method_for_log,
        &uri_for_log,
        headers,
        peer_ip,
    );
    (method_for_log, uri_for_log)
}

fn log_proxy_request_started(
    proxy_request_id: u64,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
) {
    let request_content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    info!(
        proxy_request_id,
        method = %method,
        uri = %uri,
        proxy_request_started = true,
        has_body = request_may_have_body(method, headers),
        content_length = ?request_content_length,
        peer_ip = ?peer_ip,
        "openai proxy request started"
    );
}

fn build_proxy_target_url(
    state: &AppState,
    original_uri: &Uri,
    invoke_id: &str,
) -> Result<Url, Box<Response>> {
    build_proxy_upstream_url(&state.config.openai_upstream_base_url, original_uri).map_err(|err| {
        let error_text = err.to_string();
        let status = if error_text.contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
            || error_text.contains(PROXY_INVALID_REQUEST_TARGET)
            || error_text.contains("failed to parse proxy upstream url")
        {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Box::new(build_proxy_error_response(
            ProxyErrorResponse {
                status,
                message: format!("failed to build upstream url: {err}"),
                cvm_id: None,
                retry_after_secs: None,
                code: None,
                blocked_binding: None,
            },
            invoke_id,
        ))
    })
}

async fn resolve_proxy_route_context_with_timing(
    request: RouteContextRequest<'_>,
) -> Result<PoolRoutingTimeoutSettingsResolved, Response> {
    let proxy_request_id = request.proxy_request_id;
    let route_context_started = request.route_context_started;
    let runtime_timeouts = resolve_proxy_route_context(request).await?;
    debug!(
        proxy_request_id,
        route_context_elapsed = route_context_started.elapsed().as_millis() as u64,
        "proxy route context resolved"
    );
    Ok(runtime_timeouts)
}
