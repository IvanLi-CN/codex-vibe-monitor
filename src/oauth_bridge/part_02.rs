fn collect_forwardable_headers(
    headers: &HeaderMap,
    excluded_names: &[&str],
    crypto_key: Option<&[u8; 32]>,
) -> (HeaderMap, OauthForwardedHeaderSummary) {
    let connection_scoped = crate::connection_scoped_header_names(headers);
    let mut forwarded = HeaderMap::new();
    let mut forwarded_names = BTreeSet::new();
    let mut fingerprints = crypto_key.map(|_| BTreeMap::new());
    for (name, value) in headers {
        if *name == header::AUTHORIZATION
            || excluded_names
                .iter()
                .any(|candidate| name.as_str().eq_ignore_ascii_case(candidate))
            || !crate::should_forward_proxy_header(name, &connection_scoped)
            || is_internal_proxy_metadata_header(name)
        {
            continue;
        }
        forwarded.append(name.clone(), value.clone());
        let lower_name = name.as_str().to_ascii_lowercase();
        if let (Some(crypto_key), Some(header_fingerprints)) = (crypto_key, fingerprints.as_mut())
            && is_fingerprinted_oauth_header_name(lower_name.as_str())
            && !value.as_bytes().is_empty()
        {
            header_fingerprints.insert(
                lower_name.clone(),
                oauth_fingerprint_header_value(crypto_key, lower_name.as_str(), value.as_bytes()),
            );
        }
        forwarded_names.insert(lower_name);
    }
    let names = forwarded_names.into_iter().collect::<Vec<_>>();
    let prompt_cache_header_forwarded = names
        .iter()
        .any(|name| is_prompt_cache_header_name(name.as_str()));
    (
        forwarded,
        OauthForwardedHeaderSummary {
            names,
            prompt_cache_header_forwarded,
            fingerprints,
        },
    )
}

fn normalize_counted_stream_response(mut response: Response) -> Response {
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response.headers_mut().remove(header::CONNECTION);
    response
}

async fn read_counted_oauth_upstream_bytes_with_timeout(
    upstream: Response,
    total_timeout: Duration,
    started: Instant,
    phase: &str,
) -> Result<Bytes, String> {
    let Some(timeout_budget) = crate::remaining_timeout_budget(total_timeout, started.elapsed())
    else {
        return Err(oauth_upstream_timeout_message(total_timeout, phase));
    };
    match timeout(
        timeout_budget,
        axum::body::to_bytes(upstream.into_body(), usize::MAX),
    )
    .await
    {
        Ok(result) => result.map_err(|err| err.to_string()),
        Err(_) => Err(oauth_upstream_timeout_message(total_timeout, phase)),
    }
}

fn is_supported_oauth_passthrough_route(method: &Method, path: &str) -> bool {
    *method == Method::POST
        && matches!(
            path,
            "/v1/responses/compact" | "/v1/chat/completions" | "/v1/alpha/search"
        )
}

fn oauth_models_upstream_url() -> Result<Url> {
    let mut upstream_url = oauth_codex_upstream_base_url()?;
    upstream_url.set_path(&format!(
        "{}/models",
        upstream_url.path().trim_end_matches('/')
    ));
    upstream_url.set_query(Some(&format!(
        "client_version={OAUTH_CODEX_MODELS_CLIENT_VERSION}"
    )));
    Ok(upstream_url)
}

async fn request_oauth_models(
    client: &Client,
    handshake_timeout: Duration,
    response_timeout: Duration,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<(StatusCode, Bytes), OauthUpstreamResponse> {
    let upstream_url = oauth_models_upstream_url().map_err(|err| OauthUpstreamResponse {
        response: error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("invalid oauth codex models url: {err}"),
            "server_error",
        ),
        request_debug: None,
    })?;
    let request = attach_account_header(
        client
            .get(upstream_url)
            .bearer_auth(access_token)
            .header("OpenAI-Beta", "responses=experimental"),
        chatgpt_account_id,
    );
    let request_started = Instant::now();
    let upstream = match timeout(handshake_timeout, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                "oauth_upstream_unavailable",
            );
            tag_oauth_transport_failure(
                &mut response,
                crate::PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            );
            return Err(OauthUpstreamResponse {
                response,
                request_debug: None,
            });
        }
        Err(_) => {
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "oauth codex upstream handshake timed out after {}ms",
                    handshake_timeout.as_millis()
                ),
                "oauth_upstream_handshake_timeout",
            );
            tag_oauth_transport_failure(
                &mut response,
                crate::PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
            );
            return Err(OauthUpstreamResponse {
                response,
                request_debug: None,
            });
        }
    };
    let status = upstream.status();
    let bytes = read_oauth_upstream_bytes_with_timeout(
        upstream,
        response_timeout,
        request_started,
        "reading oauth codex models response",
    )
    .await
    .map_err(|err| {
        let mut response = error_response(
            StatusCode::BAD_GATEWAY,
            &format!("failed to read oauth codex models response: {err}"),
            "oauth_upstream_read_failed",
        );
        tag_oauth_transport_failure(&mut response, crate::PROXY_FAILURE_UPSTREAM_STREAM_ERROR);
        OauthUpstreamResponse {
            response,
            request_debug: None,
        }
    })?;
    Ok((status, bytes))
}

async fn oauth_models(
    client: &Client,
    handshake_timeout: Duration,
    response_timeout: Duration,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> OauthUpstreamResponse {
    let (status, bytes) = match request_oauth_models(
        client,
        handshake_timeout,
        response_timeout,
        access_token,
        chatgpt_account_id,
    )
    .await
    {
        Ok(response) => response,
        Err(response) => return response,
    };
    if !status.is_success() {
        return OauthUpstreamResponse {
            response: json_or_plain_error_response(
                status,
                &bytes,
                "oauth_upstream_rejected_request",
            ),
            request_debug: None,
        };
    }
    match transform_models_payload(&bytes) {
        Ok(value) => OauthUpstreamResponse {
            response: (status, Json(value)).into_response(),
            request_debug: None,
        },
        Err(err) => OauthUpstreamResponse {
            response: error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth codex returned malformed models payload: {err}"),
                "oauth_upstream_invalid_models",
            ),
            request_debug: None,
        },
    }
}

struct OauthResponsesRequest<'a> {
    client: &'a Client,
    headers: &'a HeaderMap,
    response_timeout: Duration,
    account_id: Option<i64>,
    access_token: &'a str,
    chatgpt_account_id: Option<&'a str>,
    installation_seed: Option<&'a [u8; 32]>,
    crypto_key: Option<&'a [u8; 32]>,
}

fn prepare_oauth_responses_request(
    request: reqwest::RequestBuilder,
    forwarded_headers: &OauthForwardedHeaderSummary,
    body: OauthUpstreamRequestBody,
    account_id: Option<i64>,
    installation_seed: Option<&[u8; 32]>,
    crypto_key: Option<&[u8; 32]>,
) -> Result<(reqwest::RequestBuilder, OauthRequestDebugInfo, bool), OauthUpstreamResponse> {
    let prepared = match body {
        OauthUpstreamRequestBody::Empty => {
            let prepared = match prepare_responses_request_body(&[], account_id, installation_seed)
            {
                Ok(value) => value,
                Err(err) => {
                    return Err(OauthUpstreamResponse {
                        response: error_response(
                            StatusCode::BAD_REQUEST,
                            &err.to_string(),
                            "invalid_request_error",
                        ),
                        request_debug: None,
                    });
                }
            };
            let request_debug = build_oauth_request_debug(
                "/v1/responses",
                &forwarded_headers,
                Some(prepared.body.as_slice()),
                prepared.rewrite.clone(),
                Some("empty"),
                Some("small_body_rewrite"),
                crypto_key,
            );
            (
                request.body(prepared.body),
                request_debug,
                prepared.wants_stream,
            )
        }
        OauthUpstreamRequestBody::Bytes(bytes) => {
            let prepared =
                match prepare_responses_request_body(&bytes, account_id, installation_seed) {
                    Ok(value) => value,
                    Err(err) => {
                        return Err(OauthUpstreamResponse {
                            response: error_response(
                                StatusCode::BAD_REQUEST,
                                &err.to_string(),
                                "invalid_request_error",
                            ),
                            request_debug: None,
                        });
                    }
                };
            let request_debug = build_oauth_request_debug(
                "/v1/responses",
                &forwarded_headers,
                Some(prepared.body.as_slice()),
                prepared.rewrite.clone(),
                Some("memory"),
                Some("small_body_rewrite"),
                crypto_key,
            );
            (
                request.body(prepared.body),
                request_debug,
                prepared.wants_stream,
            )
        }
        OauthUpstreamRequestBody::Stream {
            body,
            debug_body_prefix,
            request_is_stream,
            snapshot_kind,
        } => {
            let request_debug = build_oauth_request_debug_with_prefix(
                "/v1/responses",
                &forwarded_headers,
                debug_body_prefix.as_deref(),
                OauthResponsesRewriteSummary::default(),
                snapshot_kind.or(Some("stream")),
                Some("large_body_passthrough"),
                crypto_key,
            );
            (
                request.body(body),
                request_debug,
                request_is_stream.unwrap_or(false),
            )
        }
    };
    Ok(prepared)
}

async fn send_oauth_responses_request(
    request: reqwest::RequestBuilder,
    request_debug: OauthRequestDebugInfo,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    response_timeout: Duration,
) -> Result<(reqwest::Response, OauthRequestDebugInfo, Instant), OauthUpstreamResponse> {
    let request = request
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental");
    let request = attach_account_header(request, chatgpt_account_id);
    log_oauth_responses_request(account_id, &request_debug);
    let request_started = Instant::now();
    info!(
        account_id,
        path = "/v1/responses",
        timeout_ms = response_timeout.as_millis() as u64,
        "oauth responses request send started"
    );
    let upstream = match timeout(response_timeout, request.send()).await {
        Ok(Ok(response)) => {
            info!(
                account_id,
                path = "/v1/responses",
                upstream_status = %response.status(),
                elapsed_ms = request_started.elapsed().as_millis() as u64,
                "oauth responses request send returned upstream response"
            );
            response
        }
        Ok(Err(err)) => {
            warn!(
                account_id,
                path = "/v1/responses",
                elapsed_ms = request_started.elapsed().as_millis() as u64,
                error = %err,
                "oauth responses request send returned upstream transport error"
            );
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                "oauth_upstream_unavailable",
            );
            tag_oauth_transport_failure(
                &mut response,
                crate::PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            );
            return Err(OauthUpstreamResponse {
                response,
                request_debug: Some(request_debug),
            });
        }
        Err(_) => {
            warn!(
                account_id,
                path = "/v1/responses",
                timeout_ms = response_timeout.as_millis() as u64,
                elapsed_ms = request_started.elapsed().as_millis() as u64,
                "oauth responses request send timed out before upstream response"
            );
            let message = format!(
                "oauth codex upstream handshake timed out after {}ms",
                response_timeout.as_millis()
            );
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &message,
                "oauth_upstream_handshake_timeout",
            );
            tag_oauth_transport_failure(
                &mut response,
                crate::PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
            );
            return Err(OauthUpstreamResponse {
                response,
                request_debug: Some(request_debug),
            });
        }
    };
    Ok((upstream, request_debug, request_started))
}

fn log_oauth_responses_request(account_id: Option<i64>, request_debug: &OauthRequestDebugInfo) {
    info!(account_id, path = "/v1/responses", forwarded_header_count = request_debug.forwarded_header_names.len(), forwarded_header_names = ?request_debug.forwarded_header_names, forwarded_header_fingerprints = ?request_debug.forwarded_header_fingerprints, prompt_cache_header_forwarded = request_debug.prompt_cache_header_forwarded, fingerprint_version = request_debug.fingerprint_version, request_body_prefix_bytes = request_debug.request_body_prefix_bytes, request_body_prefix_fingerprint = request_debug.request_body_prefix_fingerprint, request_body_snapshot_kind = request_debug.request_body_snapshot_kind, responses_body_mode = request_debug.responses_body_mode, rewrite_applied = request_debug.rewrite.applied, rewrite_added_instructions = request_debug.rewrite.added_instructions, rewrite_added_store = request_debug.rewrite.added_store, rewrite_forced_stream_true = request_debug.rewrite.forced_stream_true, rewrite_removed_max_output_tokens = request_debug.rewrite.removed_max_output_tokens, rewrite_rewrote_installation_id = request_debug.rewrite.rewrote_installation_id, rewrite_removed_installation_id = request_debug.rewrite.removed_installation_id, "forwarding oauth responses request");
}

async fn oauth_responses(
    request: OauthResponsesRequest<'_>,
    body: OauthUpstreamRequestBody,
) -> OauthUpstreamResponse {
    let OauthResponsesRequest {
        client,
        headers,
        response_timeout,
        account_id,
        access_token,
        chatgpt_account_id,
        installation_seed,
        crypto_key,
    } = request;
    let upstream_url = match build_oauth_upstream_url("/responses", None) {
        Ok(url) => url,
        Err(err) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("invalid oauth codex responses url: {err}"),
                    "server_error",
                ),
                request_debug: None,
            };
        }
    };
    let (request, forwarded_headers) = copy_forwardable_headers(
        client.post(upstream_url),
        headers,
        OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES,
        crypto_key,
    );
    let (request, request_debug, wants_stream) = match prepare_oauth_responses_request(
        request,
        &forwarded_headers,
        body,
        account_id,
        installation_seed,
        crypto_key,
    ) {
        Ok(prepared) => prepared,
        Err(response) => return response,
    };
    let (upstream, request_debug, request_started) = match send_oauth_responses_request(
        request,
        request_debug,
        account_id,
        access_token,
        chatgpt_account_id,
        response_timeout,
    )
    .await
    {
        Ok(sent) => sent,
        Err(response) => return response,
    };
    complete_oauth_responses_request(
        upstream,
        request_debug,
        wants_stream,
        response_timeout,
        request_started,
    )
    .await
}

async fn complete_oauth_responses_request(
    upstream: reqwest::Response,
    request_debug: OauthRequestDebugInfo,
    wants_stream: bool,
    response_timeout: Duration,
    request_started: Instant,
) -> OauthUpstreamResponse {
    if !upstream.status().is_success() {
        let status = upstream.status();
        let bytes = match read_oauth_upstream_bytes_with_timeout(
            upstream,
            response_timeout,
            request_started,
            "reading oauth codex error response",
        )
        .await
        {
            Ok(bytes) => bytes,
            Err(err) => {
                let mut response = error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to read oauth codex error response: {err}"),
                    "oauth_upstream_read_failed",
                );
                tag_oauth_transport_failure(
                    &mut response,
                    crate::PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                );
                return OauthUpstreamResponse {
                    response,
                    request_debug: Some(request_debug),
                };
            }
        };
        return OauthUpstreamResponse {
            response: json_or_plain_error_response(
                status,
                &bytes,
                "oauth_upstream_rejected_request",
            ),
            request_debug: Some(request_debug),
        };
    }
    if wants_stream {
        return OauthUpstreamResponse {
            response: reqwest_response_to_axum_response(upstream),
            request_debug: Some(request_debug),
        };
    }
    let upstream_headers = upstream.headers().clone();
    let bytes = match read_oauth_upstream_bytes_with_timeout(
        upstream,
        response_timeout,
        request_started,
        "reading oauth codex responses stream",
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(err) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to read oauth codex responses stream: {err}"),
                    "oauth_upstream_read_failed",
                ),
                request_debug: Some(request_debug),
            };
        }
    };
    if response_headers_indicate_event_stream(&upstream_headers)
        || crate::response_payload_looks_like_sse(bytes.as_ref())
    {
        match extract_completed_response_from_sse(&bytes) {
            Ok(response_value) => OauthUpstreamResponse {
                response: (StatusCode::OK, Json(response_value)).into_response(),
                request_debug: Some(request_debug),
            },
            Err(err) => OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to decode oauth codex response stream: {err}"),
                    "oauth_upstream_invalid_response",
                ),
                request_debug: Some(request_debug),
            },
        }
    } else {
        OauthUpstreamResponse {
            response: bytes_response_from_headers(StatusCode::OK, &upstream_headers, bytes),
            request_debug: Some(request_debug),
        }
    }
}

struct OauthPassthroughRequest<'a> {
    client: &'a Client,
    method: Method,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    handshake_timeout: Duration,
    account_id: Option<i64>,
    access_token: &'a str,
    chatgpt_account_id: Option<&'a str>,
    crypto_key: Option<&'a [u8; 32]>,
}

async fn oauth_passthrough(
    request: OauthPassthroughRequest<'_>,
    body: (ReqwestBody, Option<Bytes>),
) -> OauthUpstreamResponse {
    let OauthPassthroughRequest {
        client,
        method,
        original_uri,
        headers,
        handshake_timeout,
        account_id,
        access_token,
        chatgpt_account_id,
        crypto_key,
    } = request;
    let suffix = original_uri
        .path()
        .strip_prefix("/v1")
        .unwrap_or(original_uri.path());
    let upstream_url = match build_oauth_upstream_url(suffix, original_uri.query()) {
        Ok(url) => url,
        Err(err) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("invalid oauth codex upstream url: {err}"),
                    "server_error",
                ),
                request_debug: None,
            };
        }
    };
    let mut builder = client
        .request(method, upstream_url)
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental");
    builder = attach_account_header(builder, chatgpt_account_id);
    let (builder, forwarded_headers) = copy_forwardable_headers(builder, headers, &[], crypto_key);
    let request_debug = build_oauth_request_debug(
        original_uri.path(),
        &forwarded_headers,
        body.1.as_deref(),
        OauthResponsesRewriteSummary::default(),
        None,
        None,
        crypto_key,
    );
    log_oauth_passthrough_request(account_id, original_uri.path(), &request_debug);
    let upstream = match timeout(handshake_timeout, builder.body(body.0).send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                "oauth_upstream_unavailable",
            );
            tag_oauth_transport_failure(
                &mut response,
                crate::PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            );
            return OauthUpstreamResponse {
                response,
                request_debug: Some(request_debug),
            };
        }
        Err(_) => {
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "oauth codex upstream handshake timed out after {}ms",
                    handshake_timeout.as_millis()
                ),
                "oauth_upstream_handshake_timeout",
            );
            tag_oauth_transport_failure(
                &mut response,
                crate::PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
            );
            return OauthUpstreamResponse {
                response,
                request_debug: Some(request_debug),
            };
        }
    };
    OauthUpstreamResponse {
        response: reqwest_response_to_axum_response(upstream),
        request_debug: Some(request_debug),
    }
}

fn log_oauth_passthrough_request(
    account_id: Option<i64>,
    path: &str,
    request_debug: &OauthRequestDebugInfo,
) {
    info!(account_id, path, forwarded_header_count = request_debug.forwarded_header_names.len(), forwarded_header_names = ?request_debug.forwarded_header_names, forwarded_header_fingerprints = ?request_debug.forwarded_header_fingerprints, prompt_cache_header_forwarded = request_debug.prompt_cache_header_forwarded, fingerprint_version = request_debug.fingerprint_version, request_body_prefix_bytes = request_debug.request_body_prefix_bytes, request_body_prefix_fingerprint = request_debug.request_body_prefix_fingerprint, rewrite_applied = request_debug.rewrite.applied, rewrite_added_instructions = request_debug.rewrite.added_instructions, rewrite_added_store = request_debug.rewrite.added_store, rewrite_forced_stream_true = request_debug.rewrite.forced_stream_true, rewrite_removed_max_output_tokens = request_debug.rewrite.removed_max_output_tokens, rewrite_rewrote_installation_id = request_debug.rewrite.rewrote_installation_id, rewrite_removed_installation_id = request_debug.rewrite.removed_installation_id, "forwarding oauth passthrough request");
}

fn build_oauth_upstream_url(path_suffix: &str, query: Option<&str>) -> Result<Url> {
    let mut url = oauth_codex_upstream_base_url()?;
    let base_path = url.path().trim_end_matches('/');
    let suffix = path_suffix.trim();
    let full_path = if suffix.starts_with('/') {
        format!("{base_path}{suffix}")
    } else {
        format!("{base_path}/{suffix}")
    };
    url.set_path(&full_path);
    url.set_query(query);
    Ok(url)
}

fn copy_forwardable_headers(
    mut builder: reqwest::RequestBuilder,
    headers: &HeaderMap,
    excluded_names: &[&str],
    crypto_key: Option<&[u8; 32]>,
) -> (reqwest::RequestBuilder, OauthForwardedHeaderSummary) {
    let connection_scoped = crate::connection_scoped_header_names(headers);
    let mut forwarded_names = BTreeSet::new();
    let mut fingerprints = crypto_key.map(|_| BTreeMap::new());
    for (name, value) in headers {
        if *name == header::AUTHORIZATION
            || excluded_names
                .iter()
                .any(|candidate| name.as_str().eq_ignore_ascii_case(candidate))
            || !crate::should_forward_proxy_header(name, &connection_scoped)
            || is_internal_proxy_metadata_header(name)
        {
            continue;
        }
        builder = builder.header(name, value);
        let lower_name = name.as_str().to_ascii_lowercase();
        if let (Some(crypto_key), Some(header_fingerprints)) = (crypto_key, fingerprints.as_mut())
            && is_fingerprinted_oauth_header_name(lower_name.as_str())
            && !value.as_bytes().is_empty()
        {
            header_fingerprints.insert(
                lower_name.clone(),
                oauth_fingerprint_header_value(crypto_key, lower_name.as_str(), value.as_bytes()),
            );
        }
        forwarded_names.insert(lower_name);
    }
    let names = forwarded_names.into_iter().collect::<Vec<_>>();
    let prompt_cache_header_forwarded = names
        .iter()
        .any(|name| is_prompt_cache_header_name(name.as_str()));
    (
        builder,
        OauthForwardedHeaderSummary {
            names,
            prompt_cache_header_forwarded,
            fingerprints,
        },
    )
}

fn is_internal_proxy_metadata_header(name: &header::HeaderName) -> bool {
    matches!(name.as_str(), "x-sticky-key" | "sticky-key")
}

fn is_prompt_cache_header_name(name: &str) -> bool {
    PROMPT_CACHE_HEADER_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn is_fingerprinted_oauth_header_name(name: &str) -> bool {
    OAUTH_FINGERPRINTED_HEADER_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

fn oauth_request_body_prefix_bytes(body: Option<&[u8]>) -> Option<Vec<u8>> {
    body.map(|bytes| {
        bytes[..bytes
            .len()
            .min(OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES)]
            .to_vec()
    })
}

fn oauth_fingerprint_header_value(crypto_key: &[u8; 32], name: &str, value: &[u8]) -> String {
    oauth_fingerprint_debug_value(crypto_key, "header", name.as_bytes(), value)
}

fn oauth_fingerprint_body_prefix(crypto_key: &[u8; 32], path: &str, prefix: &[u8]) -> String {
    oauth_fingerprint_debug_value(crypto_key, "body-prefix", path.as_bytes(), prefix)
}

fn oauth_fingerprint_debug_value(
    crypto_key: &[u8; 32],
    namespace: &str,
    discriminator: &[u8],
    value: &[u8],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"oauth-debug");
    hasher.update([0]);
    hasher.update(OAUTH_FINGERPRINT_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(namespace.as_bytes());
    hasher.update([0]);
    hasher.update(discriminator);
    hasher.update([0]);
    hasher.update(crypto_key);
    hasher.update([0]);
    hasher.update(value);
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn build_oauth_request_debug(
    path: &str,
    forwarded_headers: &OauthForwardedHeaderSummary,
    body: Option<&[u8]>,
    rewrite: OauthResponsesRewriteSummary,
    request_body_snapshot_kind: Option<&'static str>,
    responses_body_mode: Option<&'static str>,
    crypto_key: Option<&[u8; 32]>,
) -> OauthRequestDebugInfo {
    let body_prefix = oauth_request_body_prefix_bytes(body);
    build_oauth_request_debug_with_prefix(
        path,
        forwarded_headers,
        body_prefix.as_deref(),
        rewrite,
        request_body_snapshot_kind,
        responses_body_mode,
        crypto_key,
    )
}

fn build_oauth_request_debug_with_prefix(
    path: &str,
    forwarded_headers: &OauthForwardedHeaderSummary,
    body_prefix: Option<&[u8]>,
    rewrite: OauthResponsesRewriteSummary,
    request_body_snapshot_kind: Option<&'static str>,
    responses_body_mode: Option<&'static str>,
    crypto_key: Option<&[u8; 32]>,
) -> OauthRequestDebugInfo {
    let request_body_prefix_bytes = match (crypto_key, body_prefix.as_ref()) {
        (Some(_), Some(prefix)) => Some(prefix.len()),
        _ => None,
    };
    let request_body_prefix_fingerprint = match (crypto_key, body_prefix.as_ref()) {
        (Some(crypto_key), Some(prefix)) => {
            Some(oauth_fingerprint_body_prefix(crypto_key, path, prefix))
        }
        _ => None,
    };

    OauthRequestDebugInfo {
        fingerprint_version: crypto_key.map(|_| OAUTH_FINGERPRINT_VERSION),
        forwarded_header_names: forwarded_headers.names.clone(),
        forwarded_header_fingerprints: if crypto_key.is_some() {
            forwarded_headers.fingerprints.clone()
        } else {
            None
        },
        prompt_cache_header_forwarded: forwarded_headers.prompt_cache_header_forwarded,
        request_body_prefix_fingerprint,
        request_body_prefix_bytes,
        request_body_snapshot_kind,
        responses_body_mode,
        rewrite,
    }
}

pub(crate) fn backfill_oauth_request_debug_body_prefix(
    debug: &mut OauthRequestDebugInfo,
    path: &str,
    body: &[u8],
    crypto_key: Option<&[u8; 32]>,
) {
    let body_prefix = oauth_request_body_prefix_bytes(Some(body));
    debug.request_body_prefix_bytes = match (crypto_key, body_prefix.as_ref()) {
        (Some(_), Some(prefix)) => Some(prefix.len()),
        _ => None,
    };
    debug.request_body_prefix_fingerprint = match (crypto_key, body_prefix.as_ref()) {
        (Some(crypto_key), Some(prefix)) => Some(oauth_fingerprint_body_prefix(
            crypto_key,
            path,
            prefix.as_slice(),
        )),
        _ => None,
    };
    if crypto_key.is_some() {
        debug.fingerprint_version = Some(OAUTH_FINGERPRINT_VERSION);
    }
}

fn response_headers_indicate_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"))
}

fn bytes_response_from_headers(status: StatusCode, headers: &HeaderMap, bytes: Bytes) -> Response {
    let connection_scoped = crate::connection_scoped_header_names(headers);
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        if matches!(name.as_str(), "content-length" | "connection")
            || !crate::should_forward_proxy_header(name, &connection_scoped)
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder.body(Body::from(bytes)).unwrap_or_else(|err| {
        error_response(
            StatusCode::BAD_GATEWAY,
            &format!("failed to build oauth buffered response: {err}"),
            "oauth_stream_error",
        )
    })
}

fn attach_account_header(
    builder: reqwest::RequestBuilder,
    chatgpt_account_id: Option<&str>,
) -> reqwest::RequestBuilder {
    if let Some(account_id) = chatgpt_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder.header("ChatGPT-Account-Id", account_id)
    } else {
        builder
    }
}

fn oauth_upstream_timeout_message(total_timeout: Duration, phase: &str) -> String {
    format!(
        "request timed out after {}ms while {phase}",
        total_timeout.as_millis()
    )
}

async fn read_oauth_upstream_bytes_with_timeout(
    upstream: reqwest::Response,
    total_timeout: Duration,
    started: Instant,
    phase: &str,
) -> Result<Bytes, String> {
    let Some(timeout_budget) = crate::remaining_timeout_budget(total_timeout, started.elapsed())
    else {
        return Err(oauth_upstream_timeout_message(total_timeout, phase));
    };

    match timeout(timeout_budget, upstream.bytes()).await {
        Ok(result) => result.map_err(|err| err.to_string()),
        Err(_) => Err(oauth_upstream_timeout_message(total_timeout, phase)),
    }
}

fn reqwest_response_to_axum_response(response: reqwest::Response) -> Response {
    let status = response.status();
    let mut builder = Response::builder().status(status);
    for (name, value) in response.headers() {
        if matches!(name.as_str(), "content-length" | "connection") {
            continue;
        }
        builder = builder.header(name, value);
    }
    let stream = response
        .bytes_stream()
        .map_err(|err| std::io::Error::other(err.to_string()));
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|err| {
            error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to stream oauth codex response: {err}"),
                "oauth_stream_error",
            )
        })
}
