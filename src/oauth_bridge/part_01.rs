const OAUTH_CODEX_UPSTREAM_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub(crate) const OAUTH_CODEX_MODELS_CLIENT_VERSION: &str = "0.111.0";
const OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES: &[&str] = &[
    "content-type",
    "content-length",
    "openai-beta",
    "chatgpt-account-id",
];
const PROMPT_CACHE_HEADER_NAMES: &[&str] = &[
    "x-prompt-cache-key",
    "prompt-cache-key",
    "x-openai-prompt-cache-key",
];
const OAUTH_FINGERPRINT_VERSION: &str = "v1";
pub(crate) const OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES: usize = 64 * 1024;
const OAUTH_TRANSPORT_FAILURE_KIND_HEADER: &str = "x-codex-oauth-transport-failure";
const OAUTH_BRIDGE_SETTINGS_SINGLETON_ID: i64 = 1;
const OAUTH_INSTALLATION_ID_METADATA_KEY: &str = "x-codex-installation-id";
const OAUTH_INSTALLATION_ID_NAMESPACE: &str = "codex-installation-id:v1";
const OAUTH_FINGERPRINTED_HEADER_NAMES: &[&str] = &[
    "session_id",
    "traceparent",
    "x-client-request-id",
    "x-codex-turn-metadata",
    "originator",
];

#[cfg(test)]
static TEST_OAUTH_CODEX_UPSTREAM_BASE_URL: Lazy<StdMutex<Option<Url>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg(test)]
pub(crate) static TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

pub(crate) fn oauth_codex_upstream_base_url() -> Result<Url> {
    #[cfg(test)]
    if let Some(url) = TEST_OAUTH_CODEX_UPSTREAM_BASE_URL
        .lock()
        .expect("lock test oauth codex upstream base url")
        .clone()
    {
        return Ok(url);
    }

    Url::parse(OAUTH_CODEX_UPSTREAM_BASE_URL)
        .context("failed to parse oauth codex upstream base url")
}

#[cfg(test)]
pub(crate) async fn set_test_oauth_codex_upstream_base_url(url: Url) {
    *TEST_OAUTH_CODEX_UPSTREAM_BASE_URL
        .lock()
        .expect("lock test oauth codex upstream base url") = Some(url);
}

#[cfg(test)]
pub(crate) async fn reset_test_oauth_codex_upstream_base_url() {
    *TEST_OAUTH_CODEX_UPSTREAM_BASE_URL
        .lock()
        .expect("lock test oauth codex upstream base url") = None;
}

pub(crate) async fn load_or_init_oauth_installation_seed(pool: &Pool<Sqlite>) -> Result<[u8; 32]> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin oauth installation seed transaction")?;
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        r#"
        SELECT installation_seed
        FROM oauth_bridge_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(OAUTH_BRIDGE_SETTINGS_SINGLETON_ID)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to load oauth installation seed")?
    {
        tx.commit()
            .await
            .context("failed to commit oauth installation seed transaction")?;
        return decode_oauth_installation_seed(existing.as_str());
    }

    let mut seed = [0_u8; 32];
    OsRng.fill_bytes(&mut seed);
    let encoded = URL_SAFE_NO_PAD.encode(seed);
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO oauth_bridge_settings (id, installation_seed)
        VALUES (?1, ?2)
        "#,
    )
    .bind(OAUTH_BRIDGE_SETTINGS_SINGLETON_ID)
    .bind(encoded)
    .execute(&mut *tx)
    .await
    .context("failed to persist oauth installation seed")?;

    let stored = sqlx::query_scalar::<_, String>(
        r#"
        SELECT installation_seed
        FROM oauth_bridge_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(OAUTH_BRIDGE_SETTINGS_SINGLETON_ID)
    .fetch_one(&mut *tx)
    .await
    .context("failed to reload oauth installation seed")?;
    tx.commit()
        .await
        .context("failed to commit oauth installation seed transaction")?;
    decode_oauth_installation_seed(stored.as_str())
}

fn decode_oauth_installation_seed(raw: &str) -> Result<[u8; 32]> {
    let decoded = URL_SAFE_NO_PAD
        .decode(raw.trim())
        .context("failed to decode oauth installation seed")?;
    let bytes: [u8; 32] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("oauth installation seed must decode to 32 bytes"))?;
    Ok(bytes)
}

pub(crate) enum OauthUpstreamRequestBody {
    Empty,
    Bytes(Bytes),
    Stream {
        body: ReqwestBody,
        debug_body_prefix: Option<Bytes>,
        request_is_stream: Option<bool>,
        snapshot_kind: Option<&'static str>,
    },
}

pub(crate) enum CountedOauthUpstreamRequestBody {
    Empty,
    Bytes(Bytes),
    Stream {
        body: Body,
        debug_body_prefix: Option<Bytes>,
        request_is_stream: Option<bool>,
        snapshot_kind: Option<&'static str>,
    },
}

pub(crate) struct OauthUpstreamRequestContext<'a> {
    pub(crate) method: Method,
    pub(crate) original_uri: &'a Uri,
    pub(crate) headers: &'a HeaderMap,
    pub(crate) handshake_timeout: Duration,
    pub(crate) response_timeout: Duration,
    pub(crate) account_id: Option<i64>,
    pub(crate) access_token: &'a str,
    pub(crate) chatgpt_account_id: Option<&'a str>,
    pub(crate) installation_seed: Option<&'a [u8; 32]>,
    pub(crate) crypto_key: Option<&'a [u8; 32]>,
}

pub(crate) struct CountedOauthUpstreamRequestContext<'a> {
    pub(crate) request: OauthUpstreamRequestContext<'a>,
    pub(crate) forward_proxy_url: Option<&'a Url>,
    pub(crate) reporter: Option<crate::proxy::UpstreamTrafficReporter>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthResponsesRewriteSummary {
    pub(crate) applied: bool,
    pub(crate) added_instructions: bool,
    pub(crate) added_store: bool,
    pub(crate) forced_stream_true: bool,
    pub(crate) removed_max_output_tokens: bool,
    pub(crate) rewrote_installation_id: bool,
    pub(crate) removed_installation_id: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OauthForwardedHeaderSummary {
    pub(crate) names: Vec<String>,
    pub(crate) prompt_cache_header_forwarded: bool,
    pub(crate) fingerprints: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthRequestDebugInfo {
    pub(crate) fingerprint_version: Option<&'static str>,
    pub(crate) forwarded_header_names: Vec<String>,
    pub(crate) forwarded_header_fingerprints: Option<BTreeMap<String, String>>,
    pub(crate) prompt_cache_header_forwarded: bool,
    pub(crate) request_body_prefix_fingerprint: Option<String>,
    pub(crate) request_body_prefix_bytes: Option<usize>,
    pub(crate) request_body_snapshot_kind: Option<&'static str>,
    pub(crate) responses_body_mode: Option<&'static str>,
    pub(crate) rewrite: OauthResponsesRewriteSummary,
}

pub(crate) type OauthResponsesDebugInfo = OauthRequestDebugInfo;

pub(crate) struct OauthUpstreamResponse {
    pub(crate) response: Response,
    pub(crate) request_debug: Option<OauthRequestDebugInfo>,
}

pub(crate) fn oauth_transport_failure_kind(headers: &HeaderMap) -> Option<&'static str> {
    match headers
        .get(OAUTH_TRANSPORT_FAILURE_KIND_HEADER)
        .and_then(|value| value.to_str().ok())?
    {
        crate::PROXY_FAILURE_FAILED_CONTACT_UPSTREAM => {
            Some(crate::PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
        }
        crate::PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT => {
            Some(crate::PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
        }
        crate::PROXY_FAILURE_UPSTREAM_STREAM_ERROR => {
            Some(crate::PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        }
        _ => None,
    }
}

fn tag_oauth_transport_failure(response: &mut Response, failure_kind: &'static str) {
    response.headers_mut().insert(
        HeaderName::from_static(OAUTH_TRANSPORT_FAILURE_KIND_HEADER),
        HeaderValue::from_static(failure_kind),
    );
}

struct PreparedResponsesRequestBody {
    wants_stream: bool,
    body: Vec<u8>,
    rewrite: OauthResponsesRewriteSummary,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ClientMetadataInstallationIdRewriteSummary {
    rewrote_installation_id: bool,
    removed_installation_id: bool,
}

pub(crate) async fn send_oauth_upstream_request(
    client: &Client,
    context: OauthUpstreamRequestContext<'_>,
    body: OauthUpstreamRequestBody,
) -> OauthUpstreamResponse {
    let OauthUpstreamRequestContext {
        method,
        original_uri,
        headers,
        handshake_timeout,
        response_timeout,
        account_id,
        access_token,
        chatgpt_account_id,
        installation_seed,
        crypto_key,
    } = context;
    match original_uri.path() {
        "/v1/models" => {
            oauth_models(
                client,
                handshake_timeout,
                response_timeout,
                access_token,
                chatgpt_account_id,
            )
            .await
        }
        "/v1/responses" => {
            oauth_responses(
                OauthResponsesRequest {
                    client,
                    headers,
                    response_timeout,
                    account_id,
                    access_token,
                    chatgpt_account_id,
                    installation_seed,
                    crypto_key,
                },
                body,
            )
            .await
        }
        path if is_supported_oauth_passthrough_route(&method, path) => {
            oauth_passthrough(
                OauthPassthroughRequest {
                    client,
                    method,
                    original_uri,
                    headers,
                    handshake_timeout,
                    account_id,
                    access_token,
                    chatgpt_account_id,
                    crypto_key,
                },
                match body {
                    OauthUpstreamRequestBody::Empty => {
                        (ReqwestBody::from(Bytes::new()), Some(Bytes::new()))
                    }
                    OauthUpstreamRequestBody::Bytes(bytes) => {
                        let debug_body_prefix =
                            oauth_request_body_prefix_bytes(Some(bytes.as_ref())).map(Bytes::from);
                        (ReqwestBody::from(bytes), debug_body_prefix)
                    }
                    OauthUpstreamRequestBody::Stream {
                        body,
                        debug_body_prefix,
                        ..
                    } => (body, debug_body_prefix),
                },
            )
            .await
        }
        _ => OauthUpstreamResponse {
            response: error_response(
                StatusCode::NOT_FOUND,
                &format!(
                    "oauth upstream route is not supported: {}",
                    original_uri.path()
                ),
                "oauth_unsupported_route",
            ),
            request_debug: None,
        },
    }
}

pub(crate) async fn send_counted_oauth_upstream_request(
    context: CountedOauthUpstreamRequestContext<'_>,
    body: CountedOauthUpstreamRequestBody,
) -> OauthUpstreamResponse {
    let CountedOauthUpstreamRequestContext {
        request,
        forward_proxy_url,
        reporter,
    } = context;
    let OauthUpstreamRequestContext {
        method,
        original_uri,
        headers,
        handshake_timeout,
        response_timeout,
        account_id,
        access_token,
        chatgpt_account_id,
        installation_seed,
        crypto_key,
    } = request;
    match original_uri.path() {
        "/v1/models" => {
            counted_oauth_models(
                handshake_timeout,
                response_timeout,
                access_token,
                chatgpt_account_id,
                forward_proxy_url,
                reporter,
            )
            .await
        }
        "/v1/responses" => {
            counted_oauth_responses(
                CountedOauthUpstreamRequestContext {
                    request: OauthUpstreamRequestContext {
                        method,
                        original_uri,
                        headers,
                        handshake_timeout,
                        response_timeout,
                        account_id,
                        access_token,
                        chatgpt_account_id,
                        installation_seed,
                        crypto_key,
                    },
                    forward_proxy_url,
                    reporter,
                },
                body,
            )
            .await
        }
        path if is_supported_oauth_passthrough_route(&method, path) => {
            counted_oauth_passthrough(
                CountedOauthUpstreamRequestContext {
                    request: OauthUpstreamRequestContext {
                        method,
                        original_uri,
                        headers,
                        handshake_timeout,
                        response_timeout,
                        account_id,
                        access_token,
                        chatgpt_account_id,
                        installation_seed,
                        crypto_key,
                    },
                    forward_proxy_url,
                    reporter,
                },
                body,
            )
            .await
        }
        _ => OauthUpstreamResponse {
            response: error_response(
                StatusCode::NOT_FOUND,
                &format!(
                    "oauth upstream route is not supported: {}",
                    original_uri.path()
                ),
                "oauth_unsupported_route",
            ),
            request_debug: None,
        },
    }
}

async fn counted_oauth_models(
    handshake_timeout: Duration,
    response_timeout: Duration,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    forward_proxy_url: Option<&Url>,
    reporter: Option<crate::proxy::UpstreamTrafficReporter>,
) -> OauthUpstreamResponse {
    let mut upstream_url = match oauth_codex_upstream_base_url() {
        Ok(url) => url,
        Err(err) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("invalid oauth codex models url: {err}"),
                    "server_error",
                ),
                request_debug: None,
            };
        }
    };
    upstream_url.set_path(&format!(
        "{}/models",
        upstream_url.path().trim_end_matches('/')
    ));
    upstream_url.set_query(Some(&format!(
        "client_version={OAUTH_CODEX_MODELS_CLIENT_VERSION}"
    )));

    let mut outbound_headers = HeaderMap::new();
    insert_oauth_auth_headers(
        &mut outbound_headers,
        access_token,
        chatgpt_account_id,
        true,
    );
    let upstream = match send_counted_oauth_http_request(
        Method::GET,
        &upstream_url,
        &outbound_headers,
        Body::empty(),
        handshake_timeout,
        forward_proxy_url,
        reporter,
    )
    .await
    {
        Ok(response) => response,
        Err(response) => return response,
    };
    let status = upstream.status();
    let request_started = Instant::now();
    let bytes = match read_counted_oauth_upstream_bytes_with_timeout(
        upstream,
        response_timeout,
        request_started,
        "reading oauth codex models response",
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(err) => {
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to read oauth codex models response: {err}"),
                "oauth_upstream_read_failed",
            );
            tag_oauth_transport_failure(&mut response, crate::PROXY_FAILURE_UPSTREAM_STREAM_ERROR);
            return OauthUpstreamResponse {
                response,
                request_debug: None,
            };
        }
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

fn prepare_counted_oauth_responses_request(
    headers: &HeaderMap,
    body: CountedOauthUpstreamRequestBody,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    installation_seed: Option<&[u8; 32]>,
    crypto_key: Option<&[u8; 32]>,
) -> Result<(HeaderMap, Body, OauthRequestDebugInfo, bool), OauthUpstreamResponse> {
    let (mut outbound_headers, forwarded_headers) =
        collect_forwardable_headers(headers, OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES, crypto_key);
    let prepared = match body {
        CountedOauthUpstreamRequestBody::Empty => {
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
                Body::from(prepared.body),
                request_debug,
                prepared.wants_stream,
            )
        }
        CountedOauthUpstreamRequestBody::Bytes(bytes) => {
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
                Body::from(prepared.body),
                request_debug,
                prepared.wants_stream,
            )
        }
        CountedOauthUpstreamRequestBody::Stream {
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
            (body, request_debug, request_is_stream.unwrap_or(false))
        }
    };
    configure_counted_oauth_responses_headers(
        &mut outbound_headers,
        access_token,
        chatgpt_account_id,
    );
    log_counted_oauth_responses_request(account_id, &prepared.1);
    Ok((outbound_headers, prepared.0, prepared.1, prepared.2))
}

fn configure_counted_oauth_responses_headers(
    headers: &mut HeaderMap,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) {
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    insert_oauth_auth_headers(headers, access_token, chatgpt_account_id, true);
}

fn log_counted_oauth_responses_request(
    account_id: Option<i64>,
    request_debug: &OauthRequestDebugInfo,
) {
    info!(account_id, path = "/v1/responses", forwarded_header_count = request_debug.forwarded_header_names.len(), forwarded_header_names = ?request_debug.forwarded_header_names, request_body_prefix_bytes = request_debug.request_body_prefix_bytes, request_body_prefix_fingerprint = request_debug.request_body_prefix_fingerprint, request_body_snapshot_kind = request_debug.request_body_snapshot_kind, responses_body_mode = request_debug.responses_body_mode, "forwarding counted oauth responses request");
}

async fn counted_oauth_responses(
    context: CountedOauthUpstreamRequestContext<'_>,
    body: CountedOauthUpstreamRequestBody,
) -> OauthUpstreamResponse {
    let CountedOauthUpstreamRequestContext {
        request,
        forward_proxy_url,
        reporter,
    } = context;
    let OauthUpstreamRequestContext {
        headers,
        handshake_timeout,
        response_timeout,
        account_id,
        access_token,
        chatgpt_account_id,
        installation_seed,
        crypto_key,
        ..
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
    let (outbound_headers, request_body, request_debug, wants_stream) =
        match prepare_counted_oauth_responses_request(
            headers,
            body,
            account_id,
            access_token,
            chatgpt_account_id,
            installation_seed,
            crypto_key,
        ) {
            Ok(prepared) => prepared,
            Err(response) => return response,
        };
    let upstream = match send_counted_oauth_http_request(
        Method::POST,
        &upstream_url,
        &outbound_headers,
        request_body,
        handshake_timeout,
        forward_proxy_url,
        reporter,
    )
    .await
    {
        Ok(response) => response,
        Err(mut response) => {
            response.request_debug = Some(request_debug);
            return response;
        }
    };
    complete_counted_oauth_responses_request(
        upstream,
        request_debug,
        wants_stream,
        response_timeout,
    )
    .await
}

async fn complete_counted_oauth_responses_request(
    upstream: Response,
    request_debug: OauthRequestDebugInfo,
    wants_stream: bool,
    response_timeout: Duration,
) -> OauthUpstreamResponse {
    if !upstream.status().is_success() {
        let status = upstream.status();
        let request_started = Instant::now();
        let bytes = match read_counted_oauth_upstream_bytes_with_timeout(
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
            response: normalize_counted_stream_response(upstream),
            request_debug: Some(request_debug),
        };
    }
    let upstream_headers = upstream.headers().clone();
    let request_started = Instant::now();
    let bytes = match read_counted_oauth_upstream_bytes_with_timeout(
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

async fn counted_oauth_passthrough(
    context: CountedOauthUpstreamRequestContext<'_>,
    body: CountedOauthUpstreamRequestBody,
) -> OauthUpstreamResponse {
    let CountedOauthUpstreamRequestContext {
        request,
        forward_proxy_url,
        reporter,
    } = context;
    let OauthUpstreamRequestContext {
        method,
        original_uri,
        headers,
        handshake_timeout,
        access_token,
        chatgpt_account_id,
        crypto_key,
        ..
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
    let (mut outbound_headers, forwarded_headers) =
        collect_forwardable_headers(headers, &[], crypto_key);
    let (request_body, debug_body_prefix) = match body {
        CountedOauthUpstreamRequestBody::Empty => (Body::empty(), Some(Bytes::new())),
        CountedOauthUpstreamRequestBody::Bytes(bytes) => {
            let debug_body_prefix =
                oauth_request_body_prefix_bytes(Some(bytes.as_ref())).map(Bytes::from);
            (Body::from(bytes), debug_body_prefix)
        }
        CountedOauthUpstreamRequestBody::Stream {
            body,
            debug_body_prefix,
            ..
        } => (body, debug_body_prefix),
    };
    let request_debug = build_oauth_request_debug(
        original_uri.path(),
        &forwarded_headers,
        debug_body_prefix.as_deref(),
        OauthResponsesRewriteSummary::default(),
        None,
        None,
        crypto_key,
    );
    insert_oauth_auth_headers(
        &mut outbound_headers,
        access_token,
        chatgpt_account_id,
        true,
    );
    let upstream = match send_counted_oauth_http_request(
        method,
        &upstream_url,
        &outbound_headers,
        request_body,
        handshake_timeout,
        forward_proxy_url,
        reporter,
    )
    .await
    {
        Ok(response) => response,
        Err(mut response) => {
            response.request_debug = Some(request_debug);
            return response;
        }
    };
    OauthUpstreamResponse {
        response: normalize_counted_stream_response(upstream),
        request_debug: Some(request_debug),
    }
}

async fn send_counted_oauth_http_request(
    method: Method,
    target_url: &Url,
    headers: &HeaderMap,
    body: Body,
    handshake_timeout: Duration,
    forward_proxy_url: Option<&Url>,
    reporter: Option<crate::proxy::UpstreamTrafficReporter>,
) -> Result<Response, OauthUpstreamResponse> {
    match timeout(
        handshake_timeout,
        crate::proxy::send_counted_upstream_http_request(
            method,
            target_url,
            headers,
            body,
            forward_proxy_url,
            reporter,
        ),
    )
    .await
    {
        Ok(Ok(response)) => Ok(response.response),
        Ok(Err(err)) => {
            let failure_kind = if err.is_timeout() {
                crate::PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
            } else {
                crate::PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
            };
            let mut response = error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                if err.is_timeout() {
                    "oauth_upstream_handshake_timeout"
                } else {
                    "oauth_upstream_unavailable"
                },
            );
            tag_oauth_transport_failure(&mut response, failure_kind);
            Err(OauthUpstreamResponse {
                response,
                request_debug: None,
            })
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
            Err(OauthUpstreamResponse {
                response,
                request_debug: None,
            })
        }
    }
}

fn insert_oauth_auth_headers(
    headers: &mut HeaderMap,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    include_beta_header: bool,
) {
    if let Ok(value) = HeaderValue::from_str(&format!("Bearer {access_token}")) {
        headers.insert(header::AUTHORIZATION, value);
    }
    if include_beta_header {
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("responses=experimental"),
        );
    }
    if let Some(account_id) = chatgpt_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        && let Ok(value) = HeaderValue::from_str(account_id)
    {
        headers.insert(HeaderName::from_static("chatgpt-account-id"), value);
    }
}
