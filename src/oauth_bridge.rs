use anyhow::{Context, Result, bail};
use axum::{
    Json,
    body::{Body, Bytes},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::TryStreamExt;
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use reqwest::{Body as ReqwestBody, Client, Url};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::time::{Instant, timeout};
use tracing::{info, warn};

#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(test)]
use std::sync::Mutex as StdMutex;

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
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: OauthUpstreamRequestBody,
    handshake_timeout: Duration,
    response_timeout: Duration,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    installation_seed: Option<&[u8; 32]>,
    crypto_key: Option<&[u8; 32]>,
) -> OauthUpstreamResponse {
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
                client,
                headers,
                handshake_timeout,
                response_timeout,
                account_id,
                access_token,
                chatgpt_account_id,
                body,
                installation_seed,
                crypto_key,
            )
            .await
        }
        path if is_supported_oauth_passthrough_route(&method, path) => {
            oauth_passthrough(
                client,
                method,
                original_uri,
                headers,
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
                handshake_timeout,
                account_id,
                access_token,
                chatgpt_account_id,
                crypto_key,
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
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: CountedOauthUpstreamRequestBody,
    handshake_timeout: Duration,
    response_timeout: Duration,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    installation_seed: Option<&[u8; 32]>,
    crypto_key: Option<&[u8; 32]>,
    forward_proxy_url: Option<&Url>,
    reporter: Option<crate::proxy::UpstreamTrafficReporter>,
) -> OauthUpstreamResponse {
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
                headers,
                handshake_timeout,
                response_timeout,
                account_id,
                access_token,
                chatgpt_account_id,
                body,
                installation_seed,
                crypto_key,
                forward_proxy_url,
                reporter,
            )
            .await
        }
        path if is_supported_oauth_passthrough_route(&method, path) => {
            counted_oauth_passthrough(
                method,
                original_uri,
                headers,
                body,
                handshake_timeout,
                account_id,
                access_token,
                chatgpt_account_id,
                crypto_key,
                forward_proxy_url,
                reporter,
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

async fn counted_oauth_responses(
    headers: &HeaderMap,
    handshake_timeout: Duration,
    response_timeout: Duration,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    body: CountedOauthUpstreamRequestBody,
    installation_seed: Option<&[u8; 32]>,
    crypto_key: Option<&[u8; 32]>,
    forward_proxy_url: Option<&Url>,
    reporter: Option<crate::proxy::UpstreamTrafficReporter>,
) -> OauthUpstreamResponse {
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
    let (mut outbound_headers, forwarded_headers) =
        collect_forwardable_headers(headers, OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES, crypto_key);
    let (request_body, request_debug, wants_stream) = match body {
        CountedOauthUpstreamRequestBody::Empty => {
            let prepared = match prepare_responses_request_body(&[], account_id, installation_seed)
            {
                Ok(value) => value,
                Err(err) => {
                    return OauthUpstreamResponse {
                        response: error_response(
                            StatusCode::BAD_REQUEST,
                            &err.to_string(),
                            "invalid_request_error",
                        ),
                        request_debug: None,
                    };
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
                        return OauthUpstreamResponse {
                            response: error_response(
                                StatusCode::BAD_REQUEST,
                                &err.to_string(),
                                "invalid_request_error",
                            ),
                            request_debug: None,
                        };
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
    outbound_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    insert_oauth_auth_headers(
        &mut outbound_headers,
        access_token,
        chatgpt_account_id,
        true,
    );
    info!(
        account_id,
        path = "/v1/responses",
        forwarded_header_count = request_debug.forwarded_header_names.len(),
        forwarded_header_names = ?request_debug.forwarded_header_names,
        request_body_prefix_bytes = request_debug.request_body_prefix_bytes,
        request_body_prefix_fingerprint = request_debug.request_body_prefix_fingerprint,
        request_body_snapshot_kind = request_debug.request_body_snapshot_kind,
        responses_body_mode = request_debug.responses_body_mode,
        "forwarding counted oauth responses request"
    );
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
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: CountedOauthUpstreamRequestBody,
    handshake_timeout: Duration,
    _account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    crypto_key: Option<&[u8; 32]>,
    forward_proxy_url: Option<&Url>,
    reporter: Option<crate::proxy::UpstreamTrafficReporter>,
) -> OauthUpstreamResponse {
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

async fn oauth_models(
    client: &Client,
    handshake_timeout: Duration,
    response_timeout: Duration,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
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

    let request = client
        .get(upstream_url)
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental");
    let request = attach_account_header(request, chatgpt_account_id);
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
            return OauthUpstreamResponse {
                response,
                request_debug: None,
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
                request_debug: None,
            };
        }
    };
    let status = upstream.status();
    let bytes = match read_oauth_upstream_bytes_with_timeout(
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

async fn oauth_responses(
    client: &Client,
    headers: &HeaderMap,
    _handshake_timeout: Duration,
    response_timeout: Duration,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    body: OauthUpstreamRequestBody,
    installation_seed: Option<&[u8; 32]>,
    crypto_key: Option<&[u8; 32]>,
) -> OauthUpstreamResponse {
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
    let (request, request_debug, wants_stream) = match body {
        OauthUpstreamRequestBody::Empty => {
            let prepared = match prepare_responses_request_body(&[], account_id, installation_seed)
            {
                Ok(value) => value,
                Err(err) => {
                    return OauthUpstreamResponse {
                        response: error_response(
                            StatusCode::BAD_REQUEST,
                            &err.to_string(),
                            "invalid_request_error",
                        ),
                        request_debug: None,
                    };
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
                        return OauthUpstreamResponse {
                            response: error_response(
                                StatusCode::BAD_REQUEST,
                                &err.to_string(),
                                "invalid_request_error",
                            ),
                            request_debug: None,
                        };
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
    let request = request
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental");
    let request = attach_account_header(request, chatgpt_account_id);
    info!(
        account_id,
        path = "/v1/responses",
        forwarded_header_count = request_debug.forwarded_header_names.len(),
        forwarded_header_names = ?request_debug.forwarded_header_names,
        forwarded_header_fingerprints = ?request_debug.forwarded_header_fingerprints,
        prompt_cache_header_forwarded = request_debug.prompt_cache_header_forwarded,
        fingerprint_version = request_debug.fingerprint_version,
        request_body_prefix_bytes = request_debug.request_body_prefix_bytes,
        request_body_prefix_fingerprint = request_debug.request_body_prefix_fingerprint,
        request_body_snapshot_kind = request_debug.request_body_snapshot_kind,
        responses_body_mode = request_debug.responses_body_mode,
        rewrite_applied = request_debug.rewrite.applied,
        rewrite_added_instructions = request_debug.rewrite.added_instructions,
        rewrite_added_store = request_debug.rewrite.added_store,
        rewrite_forced_stream_true = request_debug.rewrite.forced_stream_true,
        rewrite_removed_max_output_tokens = request_debug.rewrite.removed_max_output_tokens,
        rewrite_rewrote_installation_id = request_debug.rewrite.rewrote_installation_id,
        rewrite_removed_installation_id = request_debug.rewrite.removed_installation_id,
        "forwarding oauth responses request"
    );
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
            return OauthUpstreamResponse {
                response,
                request_debug: Some(request_debug),
            };
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
            return OauthUpstreamResponse {
                response,
                request_debug: Some(request_debug),
            };
        }
    };
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

async fn oauth_passthrough(
    client: &Client,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: (ReqwestBody, Option<Bytes>),
    handshake_timeout: Duration,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    crypto_key: Option<&[u8; 32]>,
) -> OauthUpstreamResponse {
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
    info!(
        account_id,
        path = original_uri.path(),
        forwarded_header_count = request_debug.forwarded_header_names.len(),
        forwarded_header_names = ?request_debug.forwarded_header_names,
        forwarded_header_fingerprints = ?request_debug.forwarded_header_fingerprints,
        prompt_cache_header_forwarded = request_debug.prompt_cache_header_forwarded,
        fingerprint_version = request_debug.fingerprint_version,
        request_body_prefix_bytes = request_debug.request_body_prefix_bytes,
        request_body_prefix_fingerprint = request_debug.request_body_prefix_fingerprint,
        rewrite_applied = request_debug.rewrite.applied,
        rewrite_added_instructions = request_debug.rewrite.added_instructions,
        rewrite_added_store = request_debug.rewrite.added_store,
        rewrite_forced_stream_true = request_debug.rewrite.forced_stream_true,
        rewrite_removed_max_output_tokens = request_debug.rewrite.removed_max_output_tokens,
        rewrite_rewrote_installation_id = request_debug.rewrite.rewrote_installation_id,
        rewrite_removed_installation_id = request_debug.rewrite.removed_installation_id,
        "forwarding oauth passthrough request"
    );
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

fn prepare_responses_request_body(
    body: &[u8],
    account_id: Option<i64>,
    installation_seed: Option<&[u8; 32]>,
) -> Result<PreparedResponsesRequestBody> {
    let mut value: Value =
        serde_json::from_slice(body).context("request body must be valid JSON")?;
    let Value::Object(ref mut map) = value else {
        bail!("request body must be a JSON object");
    };
    let wants_stream = map.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let mut rewrite = OauthResponsesRewriteSummary::default();
    if !map.contains_key("instructions") {
        map.insert("instructions".to_string(), Value::String(String::new()));
        rewrite.added_instructions = true;
    }
    if !map.contains_key("store") {
        map.insert("store".to_string(), Value::Bool(false));
        rewrite.added_store = true;
    }
    rewrite.forced_stream_true = map.get("stream").and_then(Value::as_bool) != Some(true);
    map.insert("stream".to_string(), Value::Bool(true));
    rewrite.removed_max_output_tokens = map.remove("max_output_tokens").is_some();
    let installation_id_rewrite =
        rewrite_client_metadata_installation_id(map, account_id, installation_seed);
    rewrite.rewrote_installation_id = installation_id_rewrite.rewrote_installation_id;
    rewrite.removed_installation_id = installation_id_rewrite.removed_installation_id;
    rewrite.applied = rewrite.added_instructions
        || rewrite.added_store
        || rewrite.forced_stream_true
        || rewrite.removed_max_output_tokens
        || rewrite.rewrote_installation_id
        || rewrite.removed_installation_id;
    Ok(PreparedResponsesRequestBody {
        wants_stream,
        body: serde_json::to_vec(&value)?,
        rewrite,
    })
}

fn rewrite_client_metadata_installation_id(
    map: &mut serde_json::Map<String, Value>,
    account_id: Option<i64>,
    installation_seed: Option<&[u8; 32]>,
) -> ClientMetadataInstallationIdRewriteSummary {
    let Some(Value::Object(client_metadata)) = map.get_mut("client_metadata") else {
        return ClientMetadataInstallationIdRewriteSummary::default();
    };
    if !client_metadata.contains_key(OAUTH_INSTALLATION_ID_METADATA_KEY) {
        return ClientMetadataInstallationIdRewriteSummary::default();
    }

    match (account_id, installation_seed) {
        (Some(account_id), Some(seed)) => {
            client_metadata.insert(
                OAUTH_INSTALLATION_ID_METADATA_KEY.to_string(),
                Value::String(derive_oauth_installation_id(seed, account_id)),
            );
            ClientMetadataInstallationIdRewriteSummary {
                rewrote_installation_id: true,
                removed_installation_id: false,
            }
        }
        _ => {
            client_metadata.remove(OAUTH_INSTALLATION_ID_METADATA_KEY);
            ClientMetadataInstallationIdRewriteSummary {
                rewrote_installation_id: false,
                removed_installation_id: true,
            }
        }
    }
}

fn derive_oauth_installation_id(seed: &[u8; 32], account_id: i64) -> String {
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(seed).expect("32-byte installation seed");
    mac.update(OAUTH_INSTALLATION_ID_NAMESPACE.as_bytes());
    mac.update(b":");
    mac.update(account_id.to_string().as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut uuid_bytes = [0_u8; 16];
    uuid_bytes.copy_from_slice(&digest[..16]);
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x80;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid_bytes[0],
        uuid_bytes[1],
        uuid_bytes[2],
        uuid_bytes[3],
        uuid_bytes[4],
        uuid_bytes[5],
        uuid_bytes[6],
        uuid_bytes[7],
        uuid_bytes[8],
        uuid_bytes[9],
        uuid_bytes[10],
        uuid_bytes[11],
        uuid_bytes[12],
        uuid_bytes[13],
        uuid_bytes[14],
        uuid_bytes[15],
    )
}

fn extract_completed_response_from_sse(bytes: &[u8]) -> Result<Value> {
    let text = String::from_utf8_lossy(bytes);
    let mut pending_event: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("event:") {
            pending_event = Some(trimmed.trim_start_matches("event:").trim().to_string());
            continue;
        }
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            pending_event = None;
            continue;
        }
        let value: Value = serde_json::from_str(payload).context("invalid SSE JSON payload")?;
        let payload_type = value.get("type").and_then(Value::as_str);
        let is_completed = pending_event.as_deref() == Some("response.completed")
            || payload_type == Some("response.completed");
        let is_failed = pending_event.as_deref() == Some("response.failed")
            || payload_type == Some("response.failed")
            || payload_type == Some("error");
        if is_completed {
            if let Some(response) = value.get("response") {
                return Ok(response.clone());
            }
            return Ok(value);
        }
        if is_failed {
            let message = value
                .get("response")
                .and_then(|response| response.get("error"))
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .or_else(|| {
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("upstream reported response.failed");
            bail!(message.to_string());
        }
        pending_event = None;
    }
    bail!("stream did not include response.completed")
}

fn transform_models_payload(bytes: &[u8]) -> Result<Value> {
    let value: Value = serde_json::from_slice(bytes).context("invalid models payload")?;
    if value.get("object").and_then(Value::as_str) == Some("list") && value.get("data").is_some() {
        return Ok(value);
    }
    let Some(models) = value.get("models").and_then(Value::as_array) else {
        bail!("missing models array");
    };
    let data = models
        .iter()
        .filter_map(|entry| entry.get("slug").and_then(Value::as_str))
        .map(|slug| {
            json!({
                "id": slug,
                "object": "model",
                "created": 0,
                "owned_by": "oauth-inline-adapter",
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "object": "list", "data": data }))
}

fn summarize_error_detail(bytes: &[u8]) -> Option<String> {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes)
        && let Some(message) = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str))
    {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.chars().take(240).collect());
        }
    }
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('<') {
        None
    } else {
        Some(trimmed.chars().take(240).collect())
    }
}

fn json_or_plain_error_response(status: StatusCode, bytes: &[u8], code: &str) -> Response {
    let message = summarize_error_detail(bytes)
        .unwrap_or_else(|| format!("oauth upstream responded with {}", status.as_u16()));
    let effective_code = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .or_else(|| value.get("code").and_then(Value::as_str))
                .map(str::to_string)
        })
        .unwrap_or_else(|| code.to_string());
    error_response(status, &message, &effective_code)
}

fn error_response(status: StatusCode, message: &str, code: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "code": code,
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
