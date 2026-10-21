use super::*;

pub(crate) async fn forward_proxy_upstream(
    UpstreamProxyCapture {
        prepared:
            PreparedProxyCaptureRequest {
                state,
                proxy_request_id,
                original_uri,
                headers: _,
                capture_target,
                target_url: _,
                pool_route_active,
                runtime_timeouts: _,
                proxy_request_permit,
                downstream_request_observer,
                proxy_request_started_at,
                capture_started,
                pool_routing_reservation_key,
                requester_ip,
                request_chain_metadata,
                sticky_key,
                prompt_cache_key,
                client_attribution_context,
                proxy_settings,
                request_info,
                semantic_projection: _,
                prompt_cache_binding_constraint: _,
                encrypted_owner_auto_guard_active: _,
                prompt_cache_conversation_override: _,
                pool_attempt_trace_context: _,
                t_req_read_ms,
                t_req_parse_ms,
                invoke_id,
                occurred_at,
                mut pool_invocation_cleanup_guard,
            },
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_first_chunk_received_at,
        prefetched_first_stream_chunk_received_at,
        prefetched_stream_timeout,
        prefetched_ttfb_ms,
        oauth_responses_debug,
        attempt_already_recorded,
        final_attempt_update,
        pending_pool_attempt_record,
        deferred_pool_early_phase_cleanup_guard,
        live_pool_attempt_activity_lease,
        pending_pool_attempt_summary,
        upstream_attempt_started_at,
        upstream_attempt_started_at_utc,
        direct_http_approx,
        final_request_body_for_capture: _,
        final_requested_service_tier: _,
        final_codex_imagegen_rewrite,
        upstream_response,
        transport_bytes_live_counted,
        reservation_guard,
        image_tool_rewrite,
        mut req_raw_pending,
        upstream_status,
        rewritten_location,
        first_byte_timeout,
        stream_timeout,
    }: UpstreamProxyCapture,
) -> Result<Response, ProxyCaptureDispatchError> {
    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let response_is_event_stream = capture_target != ProxyCaptureTarget::StandaloneSearch
        && upstream_response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
    let upstream_content_encoding = upstream_response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let response_content_encoding =
        summarize_response_content_encoding(upstream_content_encoding.as_deref());
    let selected_proxy_display_name =
        resolve_invocation_proxy_display_name(selected_proxy.as_ref());
    let mut response_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        pool_account.as_ref().map(|account| account.account_id),
        pool_account
            .as_ref()
            .map(|account| account.display_name.as_str()),
        payload_summary_upstream_account_kind(pool_account.as_ref()),
        payload_summary_upstream_base_url_host(pool_account.as_ref()),
        selected_proxy_display_name.as_deref(),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_attempt_count),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
        None,
        Some(response_content_encoding.as_str()),
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        prefetched_ttfb_ms,
    );
    set_proxy_capture_record_request_compression_algorithm(
        &mut response_running_record,
        pending_pool_attempt_record
            .as_ref()
            .and_then(|pending| pending.upstream_request_compression_algorithm.as_deref()),
    );
    if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
        state.as_ref(),
        response_running_record,
    )
    .await
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast response-ready proxy capture snapshot"
        );
    }
    let mut response_builder = Response::builder().status(upstream_status);
    for (name, value) in upstream_response.headers() {
        if should_forward_proxy_header(name, &upstream_connection_scoped) {
            if name == header::LOCATION {
                if let Some(rewritten) = rewritten_location.as_deref() {
                    response_builder = response_builder.header(name, rewritten);
                }
            } else {
                response_builder = response_builder.header(name, value);
            }
        }
    }
    if let Ok(header_value) = HeaderValue::from_str(&invoke_id) {
        response_builder =
            response_builder.header(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
    }

    let state_for_task = state.clone();
    let request_info_for_task = request_info.clone();
    let client_attribution_context_for_task = client_attribution_context.clone();
    let req_raw_pending_for_task = req_raw_pending
        .take()
        .expect("request raw capture should still be pending when response stream starts");
    let invoke_id_for_task = invoke_id.clone();
    let occurred_at_for_task = occurred_at.clone();
    let upstream_content_encoding_for_task = upstream_content_encoding.clone();
    let requester_ip_for_task = requester_ip.clone();
    let request_chain_metadata_for_task = request_chain_metadata.clone();
    let sticky_key_for_task = sticky_key.clone();
    let reservation_key_for_task = pool_routing_reservation_key.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let selected_proxy_display_name_for_task = selected_proxy_display_name.clone();
    let pool_account_for_task = pool_account.clone();
    let image_tool_rewrite_for_task = image_tool_rewrite.clone();
    let oauth_responses_debug_for_task = oauth_responses_debug.clone();
    let downstream_request_observer_for_task = downstream_request_observer.clone();
    let attempt_already_recorded_for_task = attempt_already_recorded;
    let final_attempt_update_for_task = final_attempt_update;
    let direct_http_approx_for_task = direct_http_approx.clone();
    let transport_bytes_live_counted_for_task = transport_bytes_live_counted;
    let reservation_guard_for_task = reservation_guard;
    let mut pending_pool_attempt_record_for_task = pending_pool_attempt_record.clone();
    let mut deferred_pool_early_phase_cleanup_guard_for_task =
        deferred_pool_early_phase_cleanup_guard;
    let live_pool_attempt_activity_lease_for_task = live_pool_attempt_activity_lease;
    let pending_pool_attempt_summary_for_task = pending_pool_attempt_summary.clone();
    let prefetched_first_chunk_for_task = prefetched_first_chunk;
    let prefetched_first_chunk_received_at_for_task = prefetched_first_chunk_received_at;
    let mut prefetched_first_stream_chunk_received_at_for_task =
        prefetched_first_stream_chunk_received_at;
    let prefetched_stream_timeout_for_task = prefetched_stream_timeout;
    let prefetched_ttfb_ms_for_task = prefetched_ttfb_ms;
    let upstream_attempt_started_at_for_task = upstream_attempt_started_at;
    let upstream_attempt_started_at_utc_for_task = upstream_attempt_started_at_utc;
    let first_byte_timeout_for_task = first_byte_timeout;
    let request_endpoint_for_task = original_uri.path().to_string();
    let stream_timeout_for_task = stream_timeout;
    let response_is_event_stream_for_task = response_is_event_stream;
    let proxy_request_started_at_for_task = proxy_request_started_at;
    let proxy_request_permit_for_task = proxy_request_permit;
    let (tx, rx) = mpsc::channel::<Result<DownstreamResponseChunk, io::Error>>(16);
    let (downstream_body_terminal_tx, mut downstream_body_terminal_rx) =
        watch::channel(DownstreamBodyTerminalState::Open);
    if let Some(observation) = downstream_request_observer.as_ref() {
        observation.activate_reset_monitor();
    }

    let stream_invocation_cleanup_guard = Some(PoolInvocationCleanupGuard::new(
        state_for_task.clone(),
        InvocationRecoverySelector::new(invoke_id_for_task.clone(), occurred_at_for_task.clone()),
        "stream_invocation_drop_guard",
        prompt_cache_key_for_task.as_deref(),
    ));
    crate::observability::diagnostics::spawn(async move {
        let mut reservation_guard = reservation_guard_for_task;
        let _live_pool_attempt_activity_lease_for_task = live_pool_attempt_activity_lease_for_task;
        let mut stream_invocation_cleanup_guard = stream_invocation_cleanup_guard;
        let upstream_response_content_length = upstream_response.content_length();
        let diagnostic_forward = crate::observability::diagnostics::phase(
            crate::observability::diagnostics::Phase::Forward,
        );
        let mut stream = upstream_response.into_bytes_stream();
        let ttfb_started = Instant::now();
        let stream_started = Instant::now();
        let mut proxy_request_permit_for_task = Some(proxy_request_permit_for_task);
        let mut t_upstream_ttfb_ms = prefetched_ttfb_ms_for_task;
        let mut stream_started_at: Option<Instant> = None;
        let mut active_stream_timeout =
            prefetched_stream_timeout_for_task.or(stream_timeout_for_task);
        let mut response_preview = RawResponsePreviewBuffer::default();
        let mut response_raw_writer = AsyncStreamingRawPayloadWriter::new_with_expected_size(
            state_for_task.as_ref(),
            &invoke_id_for_task,
            "response",
            proxy_settings.response_body_logging_enabled,
            upstream_content_encoding_for_task.as_deref(),
            upstream_response_content_length,
        );
        // A pool attempt observes the same downstream response stream as its invocation.
        // Capture the bytes once and attach the finalized payload to both records below.
        let attempt_response_capture_enabled = proxy_settings.response_body_logging_enabled
            && pending_pool_attempt_record_for_task.is_some();
        let mut stream_response_parser =
            StreamResponsePayloadChunkParser::for_target(capture_target);
        let mut stream_response_decoder =
            IncrementalResponsePayloadDecoder::new(upstream_content_encoding_for_task.as_deref());
        let mut first_token_ms: Option<f64> = None;
        let mut nonstream_parse_buffer = (!response_is_event_stream_for_task).then(|| {
            BoundedResponseParseBuffer::new(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES)
        });
        let mut stream_error: Option<String> = None;
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut successful_terminal_forwarded = false;
        let mut downstream_body_terminal_watch_open = true;
        let mut downstream_first_byte_logged = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let mut stream_failure_origin: Option<&'static str> = None;
        let mut upstream_read_error_kind: Option<&'static str> = None;
        let mut downstream_write_error_kind: Option<&'static str> = None;
        let mut post_terminal_upstream_read_error_kind: Option<&'static str> = None;
        let mut post_terminal_upstream_read_error_message: Option<String> = None;
        let mut post_terminal_downstream_write_error_kind: Option<&'static str> = None;
        let mut post_terminal_downstream_write_error_message: Option<String> = None;
        let mut last_upstream_chunk_received_at: Option<Instant> = None;
        let mut last_downstream_forwarded_chunk_at: Option<Instant> = None;
        let mut last_upstream_chunk_gap_ms: Option<u64> = None;

        if let Some(chunk) = prefetched_first_chunk_for_task {
            let chunk_received_at =
                prefetched_first_chunk_received_at_for_task.unwrap_or_else(Instant::now);
            response_preview.append(&chunk);
            response_raw_writer.append(&chunk);
            let successful_terminal_seen_before_chunk =
                stream_response_parser.successful_terminal_seen();
            let decoded_chunk = stream_response_decoder.ingest(&chunk);
            let token_observed = stream_response_parser.ingest_bytes(&decoded_chunk);
            if should_capture_ttft(capture_target, &request_info_for_task) && token_observed {
                let observed_ms = chunk_received_at
                    .saturating_duration_since(proxy_request_started_at_for_task)
                    .as_secs_f64()
                    * 1_000.0;
                first_token_ms = Some(observed_ms);
                crate::observability::diagnostics::milestone("model_delta");
                broadcast_proxy_capture_first_token_runtime_snapshot(
                    state_for_task.as_ref(),
                    &invoke_id_for_task,
                    &occurred_at_for_task,
                    observed_ms,
                );
            }
            let successful_terminal_in_chunk = !successful_terminal_seen_before_chunk
                && stream_response_parser.successful_terminal_seen();
            if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                buffer.append(&chunk);
            }
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            if !transport_bytes_live_counted_for_task {
                state_for_task
                    .dashboard_network_speed_cache
                    .record_response_chunk_bytes(
                        &invoke_id_for_task,
                        &occurred_at_for_task,
                        pool_account_for_task
                            .as_ref()
                            .map(|account| account.account_id),
                        payload_summary_upstream_base_url_host(pool_account_for_task.as_ref())
                            .or_else(|| {
                                pool_account_for_task
                                    .is_none()
                                    .then(|| {
                                        state_for_task.config.openai_upstream_base_url.host_str()
                                    })
                                    .flatten()
                            }),
                        chunk.len(),
                        Utc::now(),
                    );
                schedule_dashboard_network_projection(state_for_task.as_ref());
            }
            stream_started_at = Some(Instant::now());
            last_upstream_chunk_received_at = Some(chunk_received_at);
            if downstream_body_available {
                if crate::observability::diagnostics::send(
                    &tx,
                    Ok(DownstreamResponseChunk {
                        bytes: chunk,
                        successful_terminal: successful_terminal_in_chunk,
                    }),
                )
                .await
                .is_err()
                {
                    downstream_body_available = false;
                    downstream_closed = true;
                    downstream_write_error_kind = Some("receiver_dropped");
                    let _ = proxy_request_permit_for_task.take();
                } else {
                    let forwarded_at = Instant::now();
                    last_downstream_forwarded_chunk_at = Some(forwarded_at);
                    if first_token_ms.is_none()
                        && should_capture_ttft(capture_target, &request_info_for_task)
                        && token_observed
                    {
                        let observed_ms = forwarded_at
                            .saturating_duration_since(proxy_request_started_at_for_task)
                            .as_secs_f64()
                            * 1_000.0;
                        first_token_ms = Some(observed_ms);
                        crate::observability::diagnostics::milestone("model_delta");
                        broadcast_proxy_capture_first_token_runtime_snapshot(
                            state_for_task.as_ref(),
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            observed_ms,
                        );
                    }
                    if !downstream_first_byte_logged {
                        downstream_first_byte_logged = true;
                        let downstream_first_byte_elapsed =
                            stream_started.elapsed().as_millis() as u64;
                        if downstream_first_byte_log_at_info(downstream_first_byte_elapsed) {
                            info!(
                                invoke_id = %invoke_id_for_task,
                                downstream_first_byte_elapsed,
                                upstream_ttfb_ms = t_upstream_ttfb_ms,
                                forwarded_bytes,
                                "openai proxy capture streamed first byte downstream"
                            );
                        } else {
                            debug!(
                                invoke_id = %invoke_id_for_task,
                                downstream_first_byte_elapsed,
                                upstream_ttfb_ms = t_upstream_ttfb_ms,
                                forwarded_bytes,
                                "openai proxy capture streamed first byte downstream"
                            );
                        }
                    }
                }
            }
        }

        loop {
            let next_chunk = if let Some(stream_started_at) = stream_started_at {
                if let Some(stream_timeout) = active_stream_timeout {
                    let Some(timeout_budget) =
                        remaining_timeout_budget(stream_timeout, stream_started_at.elapsed())
                    else {
                        let message = pool_upstream_timeout_message(
                            stream_timeout,
                            "waiting for upstream stream completion",
                        );
                        last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                            .map(|instant| instant.elapsed().as_millis() as u64);
                        if stream_response_parser.successful_terminal_seen() {
                            post_terminal_upstream_read_error_kind = Some("timeout");
                            post_terminal_upstream_read_error_message = Some(message.clone());
                        } else {
                            stream_error = Some(message.clone());
                            stream_failure_origin = Some("proxy_timeout");
                        }
                        if stream_error.is_some()
                            && downstream_body_available
                            && crate::observability::diagnostics::send(
                                &tx,
                                Err(io::Error::other(message)),
                            )
                            .await
                            .is_err()
                        {
                            downstream_body_available = false;
                            downstream_closed = true;
                            downstream_write_error_kind = Some("receiver_dropped");
                        }
                        break;
                    };
                    let next_chunk = tokio::select! {
                        changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                            if changed.is_ok() {
                                proxy_stream_observe_downstream_body_terminal(
                                    *downstream_body_terminal_rx.borrow(),
                                    &mut successful_terminal_forwarded,
                                    &mut downstream_body_available,
                                    &mut downstream_closed,
                                    &mut downstream_write_error_kind,
                                    last_upstream_chunk_received_at,
                                    &mut last_upstream_chunk_gap_ms,
                                );
                            } else {
                                downstream_body_terminal_watch_open = false;
                            }
                            continue;
                        }
                        next_chunk = timeout(timeout_budget, stream.next()) => next_chunk,
                    };
                    match next_chunk {
                        Ok(next_chunk) => next_chunk,
                        Err(_) => {
                            let message = pool_upstream_timeout_message(
                                stream_timeout,
                                "waiting for upstream stream completion",
                            );
                            last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                                .map(|instant| instant.elapsed().as_millis() as u64);
                            if stream_response_parser.successful_terminal_seen() {
                                post_terminal_upstream_read_error_kind = Some("timeout");
                                post_terminal_upstream_read_error_message = Some(message.clone());
                            } else {
                                stream_error = Some(message.clone());
                                stream_failure_origin = Some("proxy_timeout");
                            }
                            if stream_error.is_some()
                                && downstream_body_available
                                && crate::observability::diagnostics::send(
                                    &tx,
                                    Err(io::Error::other(message)),
                                )
                                .await
                                .is_err()
                            {
                                downstream_body_available = false;
                                downstream_closed = true;
                                downstream_write_error_kind = Some("receiver_dropped");
                            }
                            break;
                        }
                    }
                } else {
                    tokio::select! {
                        changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                            if changed.is_ok() {
                                proxy_stream_observe_downstream_body_terminal(
                                    *downstream_body_terminal_rx.borrow(),
                                    &mut successful_terminal_forwarded,
                                    &mut downstream_body_available,
                                    &mut downstream_closed,
                                    &mut downstream_write_error_kind,
                                    last_upstream_chunk_received_at,
                                    &mut last_upstream_chunk_gap_ms,
                                );
                            } else {
                                downstream_body_terminal_watch_open = false;
                            }
                            continue;
                        }
                        next_chunk = stream.next() => next_chunk,
                    }
                }
            } else if let Some(attempt_started_at) = upstream_attempt_started_at_for_task {
                let Some(timeout_budget) = remaining_timeout_budget(
                    first_byte_timeout_for_task,
                    attempt_started_at.elapsed(),
                ) else {
                    let message = pool_upstream_timeout_message(
                        first_byte_timeout_for_task,
                        "waiting for first upstream chunk",
                    );
                    stream_error = Some(message.clone());
                    stream_failure_origin = Some("proxy_timeout");
                    if downstream_body_available
                        && crate::observability::diagnostics::send(
                            &tx,
                            Err(io::Error::other(message)),
                        )
                        .await
                        .is_err()
                    {
                        downstream_body_available = false;
                        downstream_closed = true;
                        downstream_write_error_kind = Some("receiver_dropped");
                    }
                    break;
                };
                let next_chunk = tokio::select! {
                    changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                        if changed.is_ok() {
                            proxy_stream_observe_downstream_body_terminal(
                                *downstream_body_terminal_rx.borrow(),
                                &mut successful_terminal_forwarded,
                                &mut downstream_body_available,
                                &mut downstream_closed,
                                &mut downstream_write_error_kind,
                                last_upstream_chunk_received_at,
                                &mut last_upstream_chunk_gap_ms,
                            );
                        } else {
                            downstream_body_terminal_watch_open = false;
                        }
                        continue;
                    }
                    next_chunk = timeout(timeout_budget, stream.next()) => next_chunk,
                };
                match next_chunk {
                    Ok(next_chunk) => next_chunk,
                    Err(_) => {
                        let message = pool_upstream_timeout_message(
                            first_byte_timeout_for_task,
                            "waiting for first upstream chunk",
                        );
                        stream_error = Some(message.clone());
                        stream_failure_origin = Some("proxy_timeout");
                        if downstream_body_available
                            && crate::observability::diagnostics::send(
                                &tx,
                                Err(io::Error::other(message)),
                            )
                            .await
                            .is_err()
                        {
                            downstream_body_available = false;
                            downstream_closed = true;
                            downstream_write_error_kind = Some("receiver_dropped");
                        }
                        break;
                    }
                }
            } else {
                tokio::select! {
                    changed = downstream_body_terminal_rx.changed(), if downstream_body_terminal_watch_open => {
                        if changed.is_ok() {
                            proxy_stream_observe_downstream_body_terminal(
                                *downstream_body_terminal_rx.borrow(),
                                &mut successful_terminal_forwarded,
                                &mut downstream_body_available,
                                &mut downstream_closed,
                                &mut downstream_write_error_kind,
                                last_upstream_chunk_received_at,
                                &mut last_upstream_chunk_gap_ms,
                            );
                        } else {
                            downstream_body_terminal_watch_open = false;
                        }
                        continue;
                    }
                    next_chunk = stream.next() => next_chunk,
                }
            };
            let Some(next_chunk) = next_chunk else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    let chunk_received_at = prefetched_first_stream_chunk_received_at_for_task
                        .take()
                        .unwrap_or_else(Instant::now);
                    let gap_before_send_ms = last_downstream_forwarded_chunk_at
                        .map(|instant| instant.elapsed().as_millis() as u64);
                    if stream_started_at.is_none() {
                        t_upstream_ttfb_ms = upstream_attempt_started_at_for_task
                            .map(elapsed_ms)
                            .unwrap_or_else(|| elapsed_ms(ttfb_started));
                        stream_started_at = Some(Instant::now());
                        active_stream_timeout =
                            prefetched_stream_timeout_for_task.or(stream_timeout_for_task);
                        let mut running_record = build_running_proxy_capture_record(
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            capture_target,
                            &request_info_for_task,
                            requester_ip_for_task.as_deref(),
                            sticky_key_for_task.as_deref(),
                            prompt_cache_key_for_task.as_deref(),
                            pool_account_for_task.is_some(),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.account_id),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.display_name.as_str()),
                            payload_summary_upstream_account_kind(pool_account_for_task.as_ref()),
                            payload_summary_upstream_base_url_host(pool_account_for_task.as_ref()),
                            selected_proxy_display_name_for_task.as_deref(),
                            pool_account_for_task
                                .as_ref()
                                .map(|_| pending_pool_attempt_summary_for_task.pool_attempt_count),
                            pool_account_for_task.as_ref().map(|_| {
                                pending_pool_attempt_summary_for_task.pool_distinct_account_count
                            }),
                            None,
                            Some(response_content_encoding.as_str()),
                            t_req_read_ms,
                            t_req_parse_ms,
                            t_upstream_connect_ms,
                            t_upstream_ttfb_ms,
                        );
                        set_proxy_capture_record_request_compression_algorithm(
                            &mut running_record,
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| {
                                    pending.upstream_request_compression_algorithm.as_deref()
                                }),
                        );
                        if let Err(err) = persist_and_broadcast_proxy_capture_runtime_snapshot(
                            state_for_task.as_ref(),
                            running_record,
                        )
                        .await
                        {
                            warn!(
                                ?err,
                                invoke_id = %invoke_id_for_task,
                                "failed to broadcast first-byte proxy capture snapshot"
                            );
                        }
                    }
                    response_preview.append(&chunk);
                    response_raw_writer.append(&chunk);
                    let successful_terminal_seen_before_chunk =
                        stream_response_parser.successful_terminal_seen();
                    let decoded_chunk = stream_response_decoder.ingest(&chunk);
                    let token_observed = stream_response_parser.ingest_bytes(&decoded_chunk);
                    if should_capture_ttft(capture_target, &request_info_for_task) && token_observed
                    {
                        let observed_ms = elapsed_ms(proxy_request_started_at_for_task);
                        first_token_ms = Some(observed_ms);
                        crate::observability::diagnostics::milestone("model_delta");
                        broadcast_proxy_capture_first_token_runtime_snapshot(
                            state_for_task.as_ref(),
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            observed_ms,
                        );
                    }
                    let successful_terminal_in_chunk = !successful_terminal_seen_before_chunk
                        && stream_response_parser.successful_terminal_seen();
                    if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                        buffer.append(&chunk);
                    }
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if !transport_bytes_live_counted_for_task {
                        state_for_task
                            .dashboard_network_speed_cache
                            .record_response_chunk_bytes(
                                &invoke_id_for_task,
                                &occurred_at_for_task,
                                pool_account_for_task
                                    .as_ref()
                                    .map(|account| account.account_id),
                                payload_summary_upstream_base_url_host(
                                    pool_account_for_task.as_ref(),
                                )
                                .or_else(|| {
                                    pool_account_for_task
                                        .is_none()
                                        .then(|| {
                                            state_for_task
                                                .config
                                                .openai_upstream_base_url
                                                .host_str()
                                        })
                                        .flatten()
                                }),
                                chunk.len(),
                                Utc::now(),
                            );
                        schedule_dashboard_network_projection(state_for_task.as_ref());
                    }
                    last_upstream_chunk_received_at = Some(chunk_received_at);
                    if successful_terminal_in_chunk
                        && downstream_body_available
                        && let Some(observation) = downstream_request_observer_for_task.as_ref()
                        && let Some(write_error) = observation
                            .wait_for_write_error_window(Duration::ZERO)
                            .await
                    {
                        downstream_body_available = false;
                        downstream_closed = true;
                        downstream_write_error_kind = Some(write_error.kind);
                        last_upstream_chunk_gap_ms = gap_before_send_ms;
                        let _ = proxy_request_permit_for_task.take();
                    }
                    if downstream_body_available {
                        if crate::observability::diagnostics::send(
                            &tx,
                            Ok(DownstreamResponseChunk {
                                bytes: chunk,
                                successful_terminal: successful_terminal_in_chunk,
                            }),
                        )
                        .await
                        .is_err()
                        {
                            downstream_body_available = false;
                            if !successful_terminal_forwarded {
                                downstream_closed = true;
                                downstream_write_error_kind = Some("receiver_dropped");
                            }
                            last_upstream_chunk_gap_ms = gap_before_send_ms;
                            let _ = proxy_request_permit_for_task.take();
                        } else {
                            let forwarded_at = Instant::now();
                            last_downstream_forwarded_chunk_at = Some(forwarded_at);
                            if first_token_ms.is_none()
                                && should_capture_ttft(capture_target, &request_info_for_task)
                                && token_observed
                            {
                                let observed_ms = elapsed_ms(proxy_request_started_at_for_task);
                                first_token_ms = Some(observed_ms);
                                crate::observability::diagnostics::milestone("model_delta");
                                broadcast_proxy_capture_first_token_runtime_snapshot(
                                    state_for_task.as_ref(),
                                    &invoke_id_for_task,
                                    &occurred_at_for_task,
                                    observed_ms,
                                );
                            }
                            if !downstream_first_byte_logged {
                                downstream_first_byte_logged = true;
                                let downstream_first_byte_elapsed =
                                    stream_started.elapsed().as_millis() as u64;
                                if downstream_first_byte_log_at_info(downstream_first_byte_elapsed)
                                {
                                    info!(
                                        invoke_id = %invoke_id_for_task,
                                        downstream_first_byte_elapsed,
                                        upstream_ttfb_ms = t_upstream_ttfb_ms,
                                        forwarded_bytes,
                                        "openai proxy capture streamed first byte downstream"
                                    );
                                } else {
                                    debug!(
                                        invoke_id = %invoke_id_for_task,
                                        downstream_first_byte_elapsed,
                                        upstream_ttfb_ms = t_upstream_ttfb_ms,
                                        forwarded_bytes,
                                        "openai proxy capture streamed first byte downstream"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(err) => {
                    let msg = format!("upstream stream error: {err}");
                    stream_response_parser.flush_pending_line();
                    let read_error_kind = proxy_stream_upstream_read_error_kind(&err);
                    if stream_response_parser.successful_terminal_seen() {
                        post_terminal_upstream_read_error_kind = Some(read_error_kind);
                        post_terminal_upstream_read_error_message = Some(msg.clone());
                    } else {
                        stream_error = Some(msg.clone());
                        stream_failure_origin = Some("upstream_read");
                        upstream_read_error_kind = Some(read_error_kind);
                    }
                    last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                        .map(|instant| instant.elapsed().as_millis() as u64);
                    if stream_error.is_some()
                        && downstream_body_available
                        && crate::observability::diagnostics::send(&tx, Err(io::Error::other(msg)))
                            .await
                            .is_err()
                    {
                        downstream_body_available = false;
                        downstream_closed = true;
                        downstream_write_error_kind = Some("receiver_dropped");
                    }
                    break;
                }
            }
        }
        stream_response_parser.flush_pending_line();
        stream_response_decoder.finish();
        drop(tx);
        drop(proxy_request_permit_for_task.take());
        let downstream_terminal_grace_deadline =
            Instant::now() + PROXY_DOWNSTREAM_WRITE_ERROR_GRACE_PERIOD;
        if !downstream_closed && stream_error.is_none() {
            wait_for_downstream_body_terminal_until(
                &mut downstream_body_terminal_rx,
                downstream_terminal_grace_deadline,
                &mut successful_terminal_forwarded,
                &mut downstream_body_available,
                &mut downstream_closed,
                &mut downstream_write_error_kind,
                last_upstream_chunk_received_at,
                &mut last_upstream_chunk_gap_ms,
            )
            .await;
        }
        if !downstream_closed
            && stream_error.is_none()
            && let Some(observation) = downstream_request_observer_for_task.as_ref()
            && let Some(remaining_grace_period) =
                downstream_terminal_grace_deadline.checked_duration_since(Instant::now())
            && let Some(write_error) = observation
                .wait_for_write_error_window(remaining_grace_period)
                .await
        {
            if proxy_stream_write_error_is_post_terminal(successful_terminal_forwarded) {
                post_terminal_downstream_write_error_kind = Some(write_error.kind);
                post_terminal_downstream_write_error_message = Some(format!(
                    "downstream transport write failed after response.completed ({})",
                    write_error.kind
                ));
            } else {
                downstream_closed = true;
                downstream_write_error_kind = Some(write_error.kind);
                if last_upstream_chunk_gap_ms.is_none() {
                    last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                        .map(|instant| instant.elapsed().as_millis() as u64);
                }
            }
        }
        if let Some(observation) = downstream_request_observer_for_task.as_ref() {
            observation.finish_response_monitoring();
        }

        if let Some(guard) = diagnostic_forward {
            guard.complete();
        }
        let diagnostic_finalize = crate::observability::diagnostics::phase(
            crate::observability::diagnostics::Phase::Finalize,
        );
        let terminal_state = if stream_error.is_some() {
            PROXY_STREAM_TERMINAL_ERROR
        } else if downstream_closed {
            PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        } else {
            PROXY_STREAM_TERMINAL_COMPLETED
        };
        if let Some(err) = stream_error.as_deref() {
            warn!(
                proxy_request_id,
                terminal_state,
                error = err,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished with upstream error"
            );
        } else {
            info!(
                proxy_request_id,
                terminal_state,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished"
            );
        }

        let t_upstream_stream_ms = stream_started_at.map(elapsed_ms).unwrap_or(0.0);
        let (req_raw_for_task, resp_raw, raw_response_write_elapsed) =
            finish_proxy_raw_captures(req_raw_pending_for_task, response_raw_writer).await;
        let attempt_resp_raw = if attempt_response_capture_enabled {
            resp_raw.clone()
        } else {
            RawPayloadMeta::default()
        };
        if raw_response_write_log_at_info(raw_response_write_elapsed, resp_raw.size_bytes) {
            info!(
                invoke_id = %invoke_id_for_task,
                raw_response_write_elapsed,
                raw_response_bytes = resp_raw.size_bytes,
                raw_response_codec = raw_payload_meta_codec(&resp_raw),
                capture_strategy = if attempt_response_capture_enabled {
                    "shared_invocation_attempt"
                } else {
                    "invocation_only"
                },
                raw_response_truncated = resp_raw.truncated,
                "openai proxy capture response raw writer finished"
            );
        } else {
            debug!(
                invoke_id = %invoke_id_for_task,
                raw_response_write_elapsed,
                raw_response_bytes = resp_raw.size_bytes,
                raw_response_codec = raw_payload_meta_codec(&resp_raw),
                capture_strategy = if attempt_response_capture_enabled {
                    "shared_invocation_attempt"
                } else {
                    "invocation_only"
                },
                raw_response_truncated = resp_raw.truncated,
                "openai proxy capture response raw writer finished"
            );
        }
        let preview_bytes = response_preview.as_slice().to_vec();
        let raw_response_preview = response_preview.into_preview();
        let streamed_response_outcome = stream_response_parser.finish();
        let successful_terminal_seen_upstream = streamed_response_outcome.successful_terminal_seen;
        let preview_looks_like_sse = response_payload_looks_like_sse_after_decode(
            &preview_bytes,
            upstream_content_encoding_for_task.as_deref(),
        );
        let response_is_stream_hint = capture_target != ProxyCaptureTarget::StandaloneSearch
            && (response_is_event_stream_for_task
                || streamed_response_outcome.saw_stream_fields
                || preview_looks_like_sse);
        let resp_parse_started = Instant::now();
        let mut response_info = if response_is_stream_hint {
            if streamed_response_outcome.saw_stream_fields {
                streamed_response_outcome.response_info
            } else {
                parse_target_response_preview_payload(
                    capture_target,
                    &preview_bytes,
                    true,
                    upstream_content_encoding_for_task.as_deref(),
                )
            }
        } else {
            nonstream_parse_buffer
                .take()
                .map(|buffer| {
                    buffer.into_response_info(
                        capture_target,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
                .unwrap_or_else(|| {
                    parse_target_response_payload(
                        capture_target,
                        &preview_bytes,
                        false,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
        };
        if let Some(reason) = stream_response_decoder.failure_reason()
            && !response_info
                .usage_missing_reason
                .as_deref()
                .is_some_and(|value| value.contains("response_decode_failed:"))
        {
            let existing = response_info.usage_missing_reason.take();
            response_info.usage_missing_reason = Some(match existing {
                Some(existing) => format!("response_decode_failed:{reason};{existing}"),
                None => format!("response_decode_failed:{reason}"),
            });
        }
        let t_resp_parse_ms = elapsed_ms(resp_parse_started);

        if response_info.model.is_none() {
            response_info.model = request_info_for_task.model.clone();
        }
        if response_info.usage_missing_reason.is_none() && stream_error.is_some() {
            response_info.usage_missing_reason = Some("upstream_stream_error".to_string());
        }
        let usage_observed = proxy_stream_usage_observed(&response_info);

        let had_stream_error = stream_error.is_some();
        let had_logical_stream_failure = response_info.stream_terminal_event.is_some();
        let pure_downstream_closed = proxy_capture_is_pure_downstream_close(
            upstream_status,
            had_stream_error,
            had_logical_stream_failure,
            downstream_closed,
        );
        let failure_kind = proxy_capture_invocation_failure_kind(
            upstream_status,
            had_stream_error,
            had_logical_stream_failure,
            pure_downstream_closed,
        );

        let downstream_error_message = if downstream_closed {
            Some(format!(
                "[{}] downstream closed while streaming upstream response",
                PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
            ))
        } else {
            None
        };
        let error_message = if let Some(err) = stream_error {
            Some(format!("[{}] {err}", PROXY_FAILURE_UPSTREAM_STREAM_ERROR))
        } else if had_logical_stream_failure {
            Some(format_upstream_response_failed_message(&response_info))
        } else if !upstream_status.is_success() {
            response_info
                .upstream_error_message
                .clone()
                .or_else(|| extract_error_message_from_response_preview(&preview_bytes))
        } else if pure_downstream_closed {
            downstream_error_message.clone()
        } else {
            None
        };
        let status = proxy_capture_invocation_status(
            upstream_status,
            error_message.is_some(),
            pure_downstream_closed,
        );
        if stream_failure_origin.is_none() {
            if had_logical_stream_failure {
                stream_failure_origin = Some("logical_terminal");
            } else if pure_downstream_closed {
                stream_failure_origin = Some("downstream_write");
            } else {
                stream_failure_origin = proxy_stream_failure_origin_from_usage_reason(
                    response_info.usage_missing_reason.as_deref(),
                );
            }
        }
        let downstream_close_phase = downstream_closed.then_some(if forwarded_chunks == 0 {
            "before_first_byte"
        } else {
            "after_first_byte"
        });
        let pending_pool_attempt_terminal_reason = if pool_account_for_task.is_none() {
            None
        } else if had_stream_error {
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string())
        } else if had_logical_stream_failure {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if pure_downstream_closed {
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string())
        } else if !upstream_status.is_success() {
            Some(
                failure_kind
                    .unwrap_or(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                    .to_string(),
            )
        } else {
            None
        };
        let mut pending_pool_attempt_summary = pending_pool_attempt_summary_for_task.clone();
        pending_pool_attempt_summary.pool_attempt_terminal_reason =
            pending_pool_attempt_terminal_reason.clone();
        let selected_proxy_display_name =
            resolve_invocation_proxy_display_name(selected_proxy_for_task.as_ref());
        let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy_for_task.as_ref() {
            let forward_proxy_success = proxy_capture_response_status_is_success(
                upstream_status,
                had_stream_error,
                had_logical_stream_failure,
            );
            if attempt_already_recorded_for_task {
                final_attempt_update_for_task.unwrap_or_default()
            } else {
                record_forward_proxy_attempt(
                    state_for_task.clone(),
                    selected_proxy.clone(),
                    forward_proxy_success,
                    Some(t_upstream_connect_ms + t_upstream_ttfb_ms + t_upstream_stream_ms),
                    proxy_capture_response_failure_kind(
                        upstream_status,
                        had_stream_error,
                        had_logical_stream_failure,
                    ),
                    false,
                )
                .await
            }
        } else {
            if let Some(account) = pool_account_for_task.as_ref() {
                let pool_route_success = !upstream_status.is_client_error()
                    && proxy_capture_response_status_is_success(
                        upstream_status,
                        had_stream_error,
                        had_logical_stream_failure,
                    );
                let request_image_intent = request_info_for_task
                    .image_intent
                    .as_deref()
                    .map(crate::ImageIntent::from_str)
                    .unwrap_or(crate::ImageIntent::Unknown);
                let route_result = if pool_route_success {
                    persist_pool_route_success_then_release(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                        record_runtime_pool_route_success(
                            state_for_task.as_ref(),
                            account.account_id,
                            upstream_attempt_started_at_utc_for_task.unwrap_or_else(Utc::now),
                            sticky_key_for_task.as_deref(),
                            prompt_cache_key_for_task.as_deref(),
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .map(|pending| pending.invoke_id.as_str()),
                            request_endpoint_for_task.as_str(),
                            request_image_intent,
                            final_codex_imagegen_rewrite.as_ref(),
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| pending.attempt_id),
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| {
                                    pending.sticky_affinity_generation.or_else(|| {
                                        pending.attempt_id.and(account.sticky_affinity_generation)
                                    })
                                }),
                            request_info_for_task.model.as_deref().or_else(|| {
                                pending_pool_attempt_record_for_task
                                    .as_ref()
                                    .and_then(|pending| {
                                        pending
                                            .request_model
                                            .as_deref()
                                            .or(pending.upstream_request_model.as_deref())
                                    })
                            }),
                        ),
                    )
                    .await
                } else if pure_downstream_closed {
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    Ok(())
                } else if had_stream_error {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream stream error")
                        .to_string();
                    persist_pool_route_failure_then_release_with_guard(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                        reservation_guard.as_mut(),
                        record_runtime_pool_route_transport_failure(
                            state_for_task.as_ref(),
                            account.account_id,
                            sticky_key_for_task.as_deref(),
                            &route_message,
                            PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .map(|pending| pending.invoke_id.as_str()),
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| pending.attempt_id),
                            pending_pool_attempt_record_for_task
                                .as_ref()
                                .and_then(|pending| {
                                    pending
                                        .request_model
                                        .as_deref()
                                        .or(pending.upstream_request_model.as_deref())
                                }),
                        ),
                    )
                    .await
                } else {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream request failed")
                        .to_string();
                    if response_info_is_retryable_responses_overload(
                        upstream_status,
                        &response_info,
                    ) {
                        persist_pool_route_failure_then_release_with_guard(
                            state_for_task.as_ref(),
                            &reservation_key_for_task,
                            reservation_guard.as_mut(),
                            record_runtime_pool_route_overload_failure(
                                state_for_task.as_ref(),
                                account.account_id,
                                sticky_key_for_task.as_deref(),
                                &route_message,
                                pending_pool_attempt_record_for_task
                                    .as_ref()
                                    .map(|pending| pending.invoke_id.as_str()),
                                pending_pool_attempt_record_for_task
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                                request_info_for_task.model.as_deref().or_else(|| {
                                    pending_pool_attempt_record_for_task.as_ref().and_then(
                                        |pending| {
                                            pending
                                                .request_model
                                                .as_deref()
                                                .or(pending.upstream_request_model.as_deref())
                                        },
                                    )
                                }),
                            ),
                        )
                        .await
                    } else {
                        persist_pool_route_failure_then_release_with_guard(
                            state_for_task.as_ref(),
                            &reservation_key_for_task,
                            reservation_guard.as_mut(),
                            record_runtime_pool_route_http_failure(
                                state_for_task.as_ref(),
                                account.account_id,
                                &account.kind,
                                account.single_account_rotation_enabled,
                                sticky_key_for_task.as_deref(),
                                upstream_status,
                                &route_message,
                                None,
                                request_endpoint_for_task.as_str(),
                                request_image_intent,
                                pending_pool_attempt_record_for_task
                                    .as_ref()
                                    .and_then(|pending| pending.attempt_id),
                                pending_pool_attempt_record_for_task
                                    .as_ref()
                                    .and_then(|pending| pending.sticky_affinity_generation),
                                prompt_cache_key_for_task
                                    .as_deref()
                                    .or(request_info_for_task.sticky_key.as_deref()),
                                request_info_for_task.model.as_deref(),
                            ),
                        )
                        .await
                    }
                };
                if let Err(err) = route_result {
                    warn!(account_id = account.account_id, error = %err, "failed to record pool capture route state");
                }
            }
            ForwardProxyAttemptUpdate::default()
        };
        let mut final_attempt_persisted = false;
        if let Some(pending_attempt_record) = pending_pool_attempt_record_for_task.as_mut() {
            set_pending_pool_upstream_request_attempt_response_capture(
                pending_attempt_record,
                &attempt_resp_raw,
                Some(response_content_encoding.as_str()),
            );
            update_pending_pool_upstream_request_attempt_http_bytes(
                pending_attempt_record,
                pending_attempt_record
                    .upstream_request_logical_body_bytes
                    .and_then(|value| usize::try_from(value).ok()),
                pending_attempt_record
                    .upstream_request_transmitted_body_bytes
                    .and_then(|value| usize::try_from(value).ok()),
                pending_attempt_record
                    .upstream_request_header_bytes_approx
                    .and_then(|value| usize::try_from(value).ok()),
                Some(forwarded_bytes),
                pending_attempt_record
                    .upstream_response_header_bytes_approx
                    .and_then(|value| usize::try_from(value).ok()),
            );
            enqueue_pool_upstream_request_attempt_snapshot(
                state_for_task.as_ref(),
                pending_attempt_record,
            );
            let finished_at = shanghai_now_string();
            let attempt_status = pool_capture_attempt_status(
                upstream_status,
                had_stream_error,
                had_logical_stream_failure,
                pure_downstream_closed,
            );
            let attempt_failure_kind = if pure_downstream_closed {
                None
            } else {
                failure_kind
            };
            final_attempt_persisted = match finalize_pool_upstream_request_attempt_in_memory(
                state_for_task.as_ref(),
                pending_attempt_record,
                finished_at.as_str(),
                attempt_status,
                Some(upstream_status),
                if downstream_closed {
                    Some(upstream_status)
                } else {
                    None
                },
                attempt_failure_kind,
                (!pure_downstream_closed)
                    .then_some(error_message.as_deref())
                    .flatten(),
                downstream_error_message.as_deref(),
                Some(t_upstream_connect_ms),
                Some(t_upstream_ttfb_ms),
                Some(t_upstream_stream_ms),
                response_info.upstream_request_id.as_deref(),
                None,
                None,
            )
            .await
            {
                Ok(()) => true,
                Err(err) => {
                    warn!(
                        invoke_id = %pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to persist final pool attempt"
                    );
                    false
                }
            };
            if let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                state_for_task.as_ref(),
                &pending_attempt_record.invoke_id,
            )
            .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to broadcast final pool attempt snapshot"
                );
            }
        }
        let pool_capture_success = proxy_capture_response_status_is_success(
            upstream_status,
            had_stream_error,
            had_logical_stream_failure,
        );
        if let Some(pending_attempt_record) = pending_pool_attempt_record_for_task.as_ref()
            && pending_attempt_record.routing_source.as_deref()
                == Some(PRIORITY_HANDOFF_ROUTING_SOURCE)
        {
            if pure_downstream_closed {
                forget_priority_handoff_attempt(pending_attempt_record.attempt_id);
                forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
            } else if pool_capture_success {
                // The capture finalizer now owns the durable terminal ordering;
                // complete a successful handoff only after its row is finalized.
                complete_runtime_priority_handoff(
                    pending_attempt_record.attempt_id,
                    Some(&pending_attempt_record.invoke_id),
                    true,
                );
                if let Some(model_key) = pending_attempt_record.request_model.as_deref()
                    && let Some(generation) = priority_handoff_generation_from_audit_json(
                        pending_attempt_record
                            .routing_selection_audit_json
                            .as_deref(),
                    )
                {
                    release_priority_handoff_for_key(
                        pending_attempt_record.upstream_account_id,
                        model_key,
                        generation,
                    );
                }
                forget_priority_handoff_attempt(pending_attempt_record.attempt_id);
                forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
            } else {
                forget_priority_handoff_attempt(pending_attempt_record.attempt_id);
                forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
            }
        }
        if pool_capture_success
            && let (Some(prompt_cache_key), Some(account)) = (
                prompt_cache_key_for_task.as_deref(),
                pool_account_for_task.as_ref(),
            )
            && (request_info_for_task.contains_encrypted_content
                || response_info.contains_encrypted_content)
        {
            match confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
                state_for_task.as_ref(),
                prompt_cache_key,
                account.account_id,
            )
            .await
            {
                Ok(true) => {
                    if let Err(err) =
                        promote_prompt_cache_group_binding_to_upstream_account_and_broadcast(
                            state_for_task.as_ref(),
                            prompt_cache_key,
                            account.account_id,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = %invoke_id_for_task,
                            prompt_cache_key,
                            account_id = account.account_id,
                            error = %err,
                            "failed to promote prompt cache group binding after encrypted session success"
                        );
                    }
                }
                Ok(false) => {}
                Err(err) => {
                    warn!(
                        invoke_id = %invoke_id_for_task,
                        prompt_cache_key,
                        account_id = account.account_id,
                        error = %err,
                        "failed to persist encrypted session owner"
                    );
                }
            }
        }
        let (billing_service_tier, pricing_mode) =
            resolve_proxy_billing_service_tier_and_pricing_mode_for_model_and_account(
                response_info.model.as_deref(),
                None,
                request_info_for_task.requested_service_tier.as_deref(),
                response_info.service_tier.as_deref(),
                pool_account_for_task.as_ref(),
            );
        let (cost_breakdown, cost_estimated, price_version) =
            estimate_proxy_cost_breakdown_from_shared_catalog(
                &state_for_task.pricing_catalog,
                response_info.model.as_deref(),
                &response_info.usage,
                billing_service_tier.as_deref(),
                pricing_mode,
            )
            .await;
        let cost = cost_breakdown.map(ProxyCostBreakdown::total);
        let request_chain_metadata_for_payload = (had_stream_error
            || had_logical_stream_failure
            || pure_downstream_closed
            || !upstream_status.is_success())
        .then_some(&request_chain_metadata_for_task);
        let payload = with_proxy_stream_terminal_diagnostics(
            with_codex_imagegen_rewrite_payload_summary(
                with_image_tool_rewrite_payload_summary(
                    build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: upstream_status,
                        is_stream: request_info_for_task.is_stream,
                        request_contains_encrypted_content: request_info_for_task
                            .contains_encrypted_content,
                        response_contains_encrypted_content: response_info
                            .contains_encrypted_content,
                        compaction_request_kind: request_info_for_task.compaction_request_kind,
                        compaction_response_kind: resolve_compaction_response_kind_for_payload(
                            capture_target,
                            response_info.compaction_response_kind,
                        ),
                        image_intent: request_info_for_task.image_intent.as_deref(),
                        request_model: request_info_for_task.model.as_deref(),
                        requested_service_tier: request_info_for_task
                            .requested_service_tier
                            .as_deref(),
                        billing_service_tier: billing_service_tier.as_deref(),
                        reasoning_effort: request_info_for_task.reasoning_effort.as_deref(),
                        response_model: response_info.model.as_deref(),
                        usage_missing_reason: response_info.usage_missing_reason.as_deref(),
                        request_parse_error: request_info_for_task.parse_error.as_deref(),
                        request_compression_algorithm: pending_pool_attempt_record_for_task
                            .as_ref()
                            .and_then(|pending| {
                                pending.upstream_request_compression_algorithm.as_deref()
                            })
                            .or_else(|| {
                                pool_account_for_task
                                    .is_none()
                                    .then(|| {
                                        direct_http_approx_for_task
                                            .request_compression
                                            .as_ref()
                                            .map(|value| value.algorithm.as_str())
                                    })
                                    .flatten()
                            }),
                        request_compression_mode: if pool_account_for_task.is_none() {
                            direct_http_approx_for_task
                                .request_compression
                                .as_ref()
                                .map(|value| value.mode.as_str())
                        } else {
                            None
                        },
                        request_compression_logical_body_bytes: if pool_account_for_task.is_none() {
                            direct_http_approx_for_task
                                .request_compression
                                .as_ref()
                                .map(|value| value.logical_body_bytes)
                        } else {
                            None
                        },
                        request_compression_transmitted_body_bytes: if pool_account_for_task
                            .is_none()
                        {
                            direct_http_approx_for_task
                                .request_compression
                                .as_ref()
                                .map(|value| value.transmitted_body_bytes)
                        } else {
                            None
                        },
                        request_compression_transmission_complete: pool_account_for_task
                            .is_none()
                            .then_some(direct_http_approx_for_task.request_transmission_complete),
                        failure_kind,
                        requester_ip: requester_ip_for_task.as_deref(),
                        request_user_agent: request_chain_metadata_for_payload
                            .and_then(|metadata| metadata.user_agent.as_deref()),
                        request_x_forwarded_for: request_chain_metadata_for_payload
                            .and_then(|metadata| metadata.x_forwarded_for.as_deref()),
                        request_forwarded: request_chain_metadata_for_payload
                            .and_then(|metadata| metadata.forwarded.as_deref()),
                        request_x_real_ip: request_chain_metadata_for_payload
                            .and_then(|metadata| metadata.x_real_ip.as_deref()),
                        upstream_scope: if pool_account_for_task.is_some() {
                            INVOCATION_UPSTREAM_SCOPE_INTERNAL
                        } else {
                            INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                        },
                        route_mode: if pool_account_for_task.is_some() {
                            INVOCATION_ROUTE_MODE_POOL
                        } else {
                            INVOCATION_ROUTE_MODE_FORWARD_PROXY
                        },
                        sticky_key: sticky_key_for_task.as_deref(),
                        prompt_cache_key: prompt_cache_key_for_task.as_deref(),
                        prompt_cache_key_attribution_source: request_info_for_task
                            .prompt_cache_key_attribution_source
                            .as_deref(),
                        client_fingerprint: client_attribution_context_for_task
                            .fingerprint
                            .as_deref(),
                        client_header_fingerprints: Some(
                            &client_attribution_context_for_task.header_fingerprints,
                        )
                        .filter(|fingerprints| !fingerprints.is_empty()),
                        upstream_account_id: pool_account_for_task
                            .as_ref()
                            .map(|account| account.account_id),
                        upstream_account_name: pool_account_for_task
                            .as_ref()
                            .map(|account| account.display_name.as_str()),
                        upstream_account_kind: payload_summary_upstream_account_kind(
                            pool_account_for_task.as_ref(),
                        ),
                        upstream_base_url_host: payload_summary_upstream_base_url_host(
                            pool_account_for_task.as_ref(),
                        ),
                        oauth_account_header_attached: oauth_account_header_attached_for_account(
                            pool_account_for_task.as_ref(),
                        ),
                        oauth_account_id_shape: oauth_account_id_shape_for_account(
                            pool_account_for_task.as_ref(),
                        ),
                        oauth_forwarded_header_count: oauth_responses_debug_for_task
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.len()),
                        oauth_forwarded_header_names: oauth_responses_debug_for_task
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.as_slice()),
                        oauth_fingerprint_version: oauth_responses_debug_for_task
                            .as_ref()
                            .and_then(|debug| debug.fingerprint_version),
                        oauth_forwarded_header_fingerprints: oauth_responses_debug_for_task
                            .as_ref()
                            .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
                        oauth_prompt_cache_header_forwarded: oauth_responses_debug_for_task
                            .as_ref()
                            .map(|debug| debug.prompt_cache_header_forwarded),
                        oauth_request_body_prefix_fingerprint: oauth_responses_debug_for_task
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
                        oauth_request_body_prefix_bytes: oauth_responses_debug_for_task
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_bytes),
                        oauth_request_body_snapshot_kind: oauth_responses_debug_for_task
                            .as_ref()
                            .and_then(|debug| debug.request_body_snapshot_kind),
                        oauth_responses_body_mode: oauth_responses_debug_for_task
                            .as_ref()
                            .and_then(|debug| debug.responses_body_mode),
                        oauth_responses_rewrite: oauth_responses_debug_for_task
                            .as_ref()
                            .map(|debug| &debug.rewrite),
                        service_tier: response_info.service_tier.as_deref(),
                        stream_terminal_event: response_info.stream_terminal_event.as_deref(),
                        upstream_error_code: response_info.upstream_error_code.as_deref(),
                        upstream_error_message: response_info.upstream_error_message.as_deref(),
                        downstream_status_code: if downstream_closed {
                            Some(upstream_status)
                        } else {
                            None
                        },
                        downstream_error_message: downstream_error_message.as_deref(),
                        upstream_request_id: response_info.upstream_request_id.as_deref(),
                        response_content_encoding: Some(response_content_encoding.as_str()),
                        stream_failure_origin,
                        upstream_read_error_kind,
                        content_encoding_chain: Some(response_content_encoding.as_str()),
                        forwarded_chunk_count: Some(forwarded_chunks),
                        forwarded_bytes: Some(forwarded_bytes),
                        usage_observed: Some(usage_observed),
                        downstream_close_phase,
                        downstream_write_error_kind,
                        last_upstream_chunk_gap_ms,
                        upstream_approx_upload_bytes: pool_account_for_task
                            .is_none()
                            .then_some(direct_http_approx_for_task.approx_upload_bytes),
                        upstream_approx_download_bytes: pool_account_for_task.is_none().then_some(
                            direct_http_approx_for_task
                                .approx_download_bytes_before_response_body
                                .saturating_add(forwarded_bytes),
                        ),
                        proxy_display_name: selected_proxy_display_name.as_deref(),
                        proxy_weight_delta: if selected_proxy_for_task.is_some() {
                            proxy_attempt_update.delta()
                        } else {
                            None
                        },
                        pool_attempt_count: pool_account_for_task
                            .as_ref()
                            .map(|_| pending_pool_attempt_summary.pool_attempt_count),
                        pool_distinct_account_count: pool_account_for_task
                            .as_ref()
                            .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
                        pool_attempt_terminal_reason: pool_account_for_task
                            .as_ref()
                            .and(pending_pool_attempt_terminal_reason.as_deref()),
                        blocked_binding: None,
                    }),
                    image_tool_rewrite_for_task.as_ref(),
                ),
                final_codex_imagegen_rewrite.as_ref(),
            ),
            successful_terminal_seen_upstream.then_some("completed"),
            post_terminal_upstream_read_error_kind,
            post_terminal_upstream_read_error_message.as_deref(),
            post_terminal_downstream_write_error_kind,
            post_terminal_downstream_write_error_message.as_deref(),
        );

        let record = ProxyCaptureRecord {
            invoke_id: invoke_id_for_task,
            occurred_at: occurred_at_for_task,
            model: response_info.model,
            usage: response_info.usage,
            cost,
            cost_breakdown,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind: failure_kind.map(|kind| kind.to_string()),
            payload: Some(payload),
            raw_response: raw_response_preview,
            response_body_preview_enabled: proxy_settings.response_body_logging_enabled,
            req_raw: req_raw_for_task,
            resp_raw,
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms: 0.0,
            },
        };

        let terminal_invocation_persisted = if let Err(err) =
            persist_and_broadcast_proxy_capture(state_for_task.as_ref(), capture_started, record)
                .await
        {
            warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            false
        } else {
            true
        };
        if terminal_invocation_persisted {
            disarm_pool_invocation_cleanup_guard(&mut stream_invocation_cleanup_guard);
        }
        if terminal_invocation_persisted
            && deferred_pool_early_phase_cleanup_guard_for_task.is_some()
        {
            finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation(
                &mut deferred_pool_early_phase_cleanup_guard_for_task,
                terminal_invocation_persisted,
            );
            if !final_attempt_persisted {
                info!(
                    proxy_request_id,
                    "terminal invocation persisted but final pool attempt state did not; disarming deferred orphan recovery to preserve the completed invocation"
                );
            }
        } else if !terminal_invocation_persisted
            && deferred_pool_early_phase_cleanup_guard_for_task.is_some()
        {
            info!(
                proxy_request_id,
                "terminal invocation did not persist; keeping deferred orphan recovery armed until the pending attempt row is repaired"
            );
        }
        if let Some(guard) = diagnostic_finalize {
            guard.complete();
        }
    });
    disarm_pool_invocation_cleanup_guard(&mut pool_invocation_cleanup_guard);

    response_builder
        .body(Body::from_stream(TrackedDownstreamReceiverStream::new(
            ReceiverStream::new(rx),
            downstream_body_terminal_tx,
        )))
        .map_err(|err| ProxyCaptureDispatchError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to build proxy response: {err}"),
            invoke_id: None,
        })
}
pub(crate) fn should_capture_ttft(
    capture_target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
) -> bool {
    if !request_info.is_stream {
        return false;
    }
    if matches!(
        capture_target,
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits
    ) {
        return false;
    }
    !matches!(
        request_info
            .image_intent
            .as_deref()
            .map(ImageIntent::from_str),
        Some(ImageIntent::Yes | ImageIntent::DirectImage)
    )
}

pub(crate) fn request_body_size_bucket(bytes: usize) -> &'static str {
    match bytes {
        0 => "empty",
        1..=4096 => "le_4k",
        4097..=65536 => "le_64k",
        65537..=1048576 => "le_1m",
        1048577..=8388608 => "le_8m",
        _ => "gt_8m",
    }
}

pub(crate) fn capture_request_body_read_log_at_info(bytes: usize, elapsed_ms: f64) -> bool {
    bytes >= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES || elapsed_ms >= 1_000.0
}

pub(crate) fn downstream_first_byte_log_at_info(elapsed_ms: u64) -> bool {
    elapsed_ms >= 2_000
}

pub(crate) fn raw_response_write_log_at_info(elapsed_ms: u64, bytes: i64) -> bool {
    elapsed_ms >= 500 || bytes >= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES as i64
}

pub(crate) fn resolve_compaction_response_kind_for_payload(
    capture_target: ProxyCaptureTarget,
    parsed_kind: Option<CompactionKind>,
) -> Option<CompactionKind> {
    if capture_target == ProxyCaptureTarget::StandaloneSearch {
        None
    } else if capture_target == ProxyCaptureTarget::ResponsesCompact {
        Some(CompactionKind::Compact)
    } else {
        parsed_kind
    }
}
