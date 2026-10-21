use super::*;

pub(crate) async fn send_forward_proxy_request_with_429_retry(
    state: Arc<AppState>,
    method: Method,
    target_url: Url,
    headers: &HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    live_reporter: Option<UpstreamTrafficReporter>,
    handshake_timeout: Duration,
    capture_target: Option<ProxyCaptureTarget>,
    upstream_429_max_retries: u8,
) -> Result<ForwardProxyUpstreamResponse, ForwardProxyUpstreamError> {
    async fn drain_proxy_response_body_bytes(response: ProxyUpstreamResponseBody) -> usize {
        let mut total = 0usize;
        let mut stream = response.into_bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    total = total.saturating_add(bytes.len());
                }
                Err(err) => {
                    warn!(error = %err, "failed to drain retry response body for byte accounting");
                    break;
                }
            }
        }
        total
    }

    let request_connection_scoped = connection_scoped_header_names(headers);
    let request_body_len = body.as_ref().map(pool_request_snapshot_body_bytes);
    let request_compression = body.as_ref().and_then(|snapshot| match snapshot {
        PoolReplayBodySnapshot::Memory(body_bytes) => observe_request_compression_from_bytes(
            body_bytes,
            headers
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok()),
        ),
        _ => None,
    });
    let mut approx_upload_bytes = 0usize;
    let mut approx_download_bytes_before_response_body = 0usize;

    for attempt in 0..=upstream_429_max_retries {
        let selected_proxy = match select_forward_proxy_for_request(state.as_ref()).await {
            Ok(selected_proxy) => selected_proxy,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy: SelectedForwardProxy::from_endpoint(
                        &ForwardProxyEndpoint::direct(),
                    ),
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to select forward proxy node: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                    http_approx: ForwardProxyHttpApproxObservation::default(),
                });
            }
        };
        let mut outbound_headers = HeaderMap::new();
        for (name, value) in headers {
            if should_forward_proxy_header(name, &request_connection_scoped) {
                outbound_headers.append(name.clone(), value.clone());
            }
        }
        let transmitted_body_bytes = body
            .as_ref()
            .map(pool_request_snapshot_body_bytes)
            .unwrap_or_default();
        if let Some(body_bytes) = body.as_ref()
            && !outbound_headers.contains_key(header::CONTENT_LENGTH)
            && let Ok(content_length) =
                HeaderValue::from_str(&pool_request_snapshot_body_bytes(body_bytes).to_string())
        {
            outbound_headers.insert(header::CONTENT_LENGTH, content_length.clone());
        }
        let request_header_bytes_approx = http_visible_header_bytes_approx(&outbound_headers);

        if attempt > 0 && live_reporter.is_some() {
            state.observability.counter(
                "cvm_proxy_retries_total",
                &[
                    (
                        "endpoint",
                        crate::observability::endpoint_from_path(
                            capture_target.map(|target| target.endpoint()).unwrap_or(""),
                        ),
                    ),
                    ("reason", "rate_limit"),
                ],
                1,
            );
        }
        let connect_started = Instant::now();
        let response = match timeout(
            handshake_timeout,
            send_counted_upstream_http_request(
                method.clone(),
                &target_url,
                &outbound_headers,
                body.as_ref()
                    .map(|snapshot| {
                        counted_http_body_from_snapshot(snapshot, ObservedByteCounter::default())
                    })
                    .unwrap_or_else(Body::empty),
                selected_proxy.endpoint_url.as_ref(),
                live_reporter.clone(),
            ),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                let attempt_upload_bytes =
                    request_header_bytes_approx.saturating_add(transmitted_body_bytes);
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: err.message,
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: elapsed_ms(connect_started),
                    http_approx: ForwardProxyHttpApproxObservation {
                        approx_upload_bytes: approx_upload_bytes
                            .saturating_add(attempt_upload_bytes),
                        approx_download_bytes_before_response_body,
                        final_response_header_bytes_approx: 0,
                        request_compression: request_compression.as_ref().map(|observation| {
                            RequestCompressionObservation {
                                algorithm: observation.algorithm.clone(),
                                mode: observation.mode.clone(),
                                logical_body_bytes: observation.logical_body_bytes,
                                transmitted_body_bytes,
                            }
                        }),
                        request_transmission_complete: false,
                    },
                });
            }
            Err(_) => {
                let attempt_upload_bytes =
                    request_header_bytes_approx.saturating_add(transmitted_body_bytes);
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: proxy_request_send_timeout_message(capture_target, handshake_timeout),
                    failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT,
                    connect_latency_ms: elapsed_ms(connect_started),
                    http_approx: ForwardProxyHttpApproxObservation {
                        approx_upload_bytes: approx_upload_bytes
                            .saturating_add(attempt_upload_bytes),
                        approx_download_bytes_before_response_body,
                        final_response_header_bytes_approx: 0,
                        request_compression: request_compression.as_ref().map(|observation| {
                            RequestCompressionObservation {
                                algorithm: observation.algorithm.clone(),
                                mode: observation.mode.clone(),
                                logical_body_bytes: observation.logical_body_bytes,
                                transmitted_body_bytes,
                            }
                        }),
                        request_transmission_complete: false,
                    },
                });
            }
        };

        let connect_latency_ms = elapsed_ms(connect_started);
        let attempt_upload_bytes =
            request_header_bytes_approx.saturating_add(transmitted_body_bytes);
        approx_upload_bytes = approx_upload_bytes.saturating_add(attempt_upload_bytes);
        let response_header_bytes_approx =
            http_visible_header_bytes_approx(response.response.headers());
        approx_download_bytes_before_response_body =
            approx_download_bytes_before_response_body.saturating_add(response_header_bytes_approx);
        let request_transmission_complete =
            request_body_len.is_none_or(|len| transmitted_body_bytes == len);
        let final_request_compression =
            request_compression
                .as_ref()
                .map(|observation| RequestCompressionObservation {
                    algorithm: observation.algorithm.clone(),
                    mode: observation.mode.clone(),
                    logical_body_bytes: observation.logical_body_bytes,
                    transmitted_body_bytes,
                });
        if response.response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(ForwardProxyUpstreamResponse {
                selected_proxy,
                response: ProxyUpstreamResponseBody::Axum(response.response),
                transport_bytes_live_counted: true,
                connect_latency_ms,
                attempt_started_at: connect_started,
                attempt_recorded: false,
                attempt_update: None,
                http_approx: ForwardProxyHttpApproxObservation {
                    approx_upload_bytes,
                    approx_download_bytes_before_response_body,
                    final_response_header_bytes_approx: response_header_bytes_approx,
                    request_compression: final_request_compression,
                    request_transmission_complete,
                },
            });
        }

        if attempt < upstream_429_max_retries {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(connect_latency_ms),
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429),
                false,
            )
            .await;

            let retry_delay = response
                .response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(parse_retry_after_delay)
                .unwrap_or_else(|| {
                    fallback_proxy_429_retry_delay_for_state(state.as_ref(), u32::from(attempt) + 1)
                });
            info!(
                proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                proxy_source = selected_proxy.source,
                proxy_label = selected_proxy.display_name,
                proxy_url_ref = %forward_proxy_log_ref_option(selected_proxy.endpoint_url_raw.as_deref()),
                retry_index = attempt + 1,
                max_429_retries = upstream_429_max_retries,
                retry_after_ms = retry_delay.as_millis(),
                "upstream responded 429; retrying forward proxy request"
            );
            approx_download_bytes_before_response_body = approx_download_bytes_before_response_body
                .saturating_add(
                    drain_proxy_response_body_bytes(ProxyUpstreamResponseBody::Axum(
                        response.response,
                    ))
                    .await,
                );
            crate::observability::diagnostics::wait(
                crate::observability::diagnostics::Resource::RetryBackoff,
                sleep(retry_delay),
            )
            .await;
            continue;
        }

        // Final 429: defer attempt recording until the caller finishes consuming / forwarding
        // the response body, so a later stream error can override this classification.
        return Ok(ForwardProxyUpstreamResponse {
            selected_proxy,
            response: ProxyUpstreamResponseBody::Axum(response.response),
            transport_bytes_live_counted: true,
            connect_latency_ms,
            attempt_started_at: connect_started,
            attempt_recorded: false,
            attempt_update: None,
            http_approx: ForwardProxyHttpApproxObservation {
                approx_upload_bytes,
                approx_download_bytes_before_response_body,
                final_response_header_bytes_approx: response_header_bytes_approx,
                request_compression: final_request_compression,
                request_transmission_complete,
            },
        });
    }

    unreachable!("429 retry loop should always return a response or error")
}
