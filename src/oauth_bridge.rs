use anyhow::{Context, Result, bail};
use axum::{
    Json,
    body::{Body, Bytes},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use reqwest::{Body as ReqwestBody, Client, Url};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::time::Duration;
use tokio::time::{Instant, timeout};
use tracing::info;

#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(test)]
use std::sync::Mutex as StdMutex;

const OAUTH_CODEX_UPSTREAM_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const OAUTH_CODEX_MODELS_CLIENT_VERSION: &str = "0.111.0";
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

pub(crate) enum OauthUpstreamRequestBody {
    Empty,
    Bytes(Bytes),
    Stream(ReqwestBody),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthResponsesRewriteSummary {
    pub(crate) applied: bool,
    pub(crate) added_instructions: bool,
    pub(crate) added_store: bool,
    pub(crate) forced_stream_true: bool,
    pub(crate) removed_max_output_tokens: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OauthForwardedHeaderSummary {
    pub(crate) names: Vec<String>,
    pub(crate) prompt_cache_header_forwarded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OauthResponsesDebugInfo {
    pub(crate) forwarded_header_names: Vec<String>,
    pub(crate) prompt_cache_header_forwarded: bool,
    pub(crate) rewrite: OauthResponsesRewriteSummary,
}

pub(crate) struct OauthUpstreamResponse {
    pub(crate) response: Response,
    pub(crate) responses_debug: Option<OauthResponsesDebugInfo>,
}

struct PreparedResponsesRequestBody {
    wants_stream: bool,
    body: Vec<u8>,
    rewrite: OauthResponsesRewriteSummary,
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
                match body {
                    OauthUpstreamRequestBody::Empty => Bytes::new(),
                    OauthUpstreamRequestBody::Bytes(bytes) => bytes,
                    OauthUpstreamRequestBody::Stream(_) => {
                        return OauthUpstreamResponse {
                            response: error_response(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "streamed request bodies are not supported for /v1/responses",
                                "server_error",
                            ),
                            responses_debug: None,
                        };
                    }
                },
            )
            .await
        }
        path if is_supported_oauth_passthrough_route(path) => {
            oauth_passthrough(
                client,
                method,
                original_uri,
                headers,
                match body {
                    OauthUpstreamRequestBody::Empty => ReqwestBody::from(Bytes::new()),
                    OauthUpstreamRequestBody::Bytes(bytes) => ReqwestBody::from(bytes),
                    OauthUpstreamRequestBody::Stream(body) => body,
                },
                handshake_timeout,
                access_token,
                chatgpt_account_id,
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
            responses_debug: None,
        },
    }
}

fn is_supported_oauth_passthrough_route(path: &str) -> bool {
    matches!(path, "/v1/responses/compact" | "/v1/chat/completions")
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
                responses_debug: None,
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
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to contact oauth codex upstream: {err}"),
                    "oauth_upstream_unavailable",
                ),
                responses_debug: None,
            };
        }
        Err(_) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!(
                        "oauth codex upstream handshake timed out after {}ms",
                        handshake_timeout.as_millis()
                    ),
                    "oauth_upstream_handshake_timeout",
                ),
                responses_debug: None,
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
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to read oauth codex models response: {err}"),
                    "oauth_upstream_read_failed",
                ),
                responses_debug: None,
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
            responses_debug: None,
        };
    }
    match transform_models_payload(&bytes) {
        Ok(value) => OauthUpstreamResponse {
            response: (status, Json(value)).into_response(),
            responses_debug: None,
        },
        Err(err) => OauthUpstreamResponse {
            response: error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth codex returned malformed models payload: {err}"),
                "oauth_upstream_invalid_models",
            ),
            responses_debug: None,
        },
    }
}

async fn oauth_responses(
    client: &Client,
    headers: &HeaderMap,
    handshake_timeout: Duration,
    response_timeout: Duration,
    account_id: Option<i64>,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    body: Bytes,
) -> OauthUpstreamResponse {
    let prepared = match prepare_responses_request_body(&body) {
        Ok(value) => value,
        Err(err) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_REQUEST,
                    &err.to_string(),
                    "invalid_request_error",
                ),
                responses_debug: None,
            };
        }
    };
    let upstream_url = match build_oauth_upstream_url("/responses", None) {
        Ok(url) => url,
        Err(err) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("invalid oauth codex responses url: {err}"),
                    "server_error",
                ),
                responses_debug: None,
            };
        }
    };
    let (request, forwarded_headers) = copy_forwardable_headers(
        client.post(upstream_url),
        headers,
        OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES,
    );
    let request = request
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental")
        .body(prepared.body);
    let request = attach_account_header(request, chatgpt_account_id);
    let responses_debug = OauthResponsesDebugInfo {
        forwarded_header_names: forwarded_headers.names.clone(),
        prompt_cache_header_forwarded: forwarded_headers.prompt_cache_header_forwarded,
        rewrite: prepared.rewrite.clone(),
    };
    info!(
        account_id,
        path = "/v1/responses",
        forwarded_header_count = responses_debug.forwarded_header_names.len(),
        forwarded_header_names = ?responses_debug.forwarded_header_names,
        prompt_cache_header_forwarded = responses_debug.prompt_cache_header_forwarded,
        rewrite_applied = responses_debug.rewrite.applied,
        rewrite_added_instructions = responses_debug.rewrite.added_instructions,
        rewrite_added_store = responses_debug.rewrite.added_store,
        rewrite_forced_stream_true = responses_debug.rewrite.forced_stream_true,
        rewrite_removed_max_output_tokens = responses_debug.rewrite.removed_max_output_tokens,
        "forwarding oauth responses request"
    );
    let request_started = Instant::now();
    let upstream = match timeout(handshake_timeout, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to contact oauth codex upstream: {err}"),
                    "oauth_upstream_unavailable",
                ),
                responses_debug: Some(responses_debug),
            };
        }
        Err(_) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!(
                        "oauth codex upstream handshake timed out after {}ms",
                        handshake_timeout.as_millis()
                    ),
                    "oauth_upstream_handshake_timeout",
                ),
                responses_debug: Some(responses_debug),
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
                return OauthUpstreamResponse {
                    response: error_response(
                        StatusCode::BAD_GATEWAY,
                        &format!("failed to read oauth codex error response: {err}"),
                        "oauth_upstream_read_failed",
                    ),
                    responses_debug: Some(responses_debug),
                };
            }
        };
        return OauthUpstreamResponse {
            response: json_or_plain_error_response(
                status,
                &bytes,
                "oauth_upstream_rejected_request",
            ),
            responses_debug: Some(responses_debug),
        };
    }
    if prepared.wants_stream {
        return OauthUpstreamResponse {
            response: reqwest_response_to_axum_response(upstream),
            responses_debug: Some(responses_debug),
        };
    }
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
                responses_debug: Some(responses_debug),
            };
        }
    };
    match extract_completed_response_from_sse(&bytes) {
        Ok(response_value) => OauthUpstreamResponse {
            response: (StatusCode::OK, Json(response_value)).into_response(),
            responses_debug: Some(responses_debug),
        },
        Err(err) => OauthUpstreamResponse {
            response: error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to decode oauth codex response stream: {err}"),
                "oauth_upstream_invalid_response",
            ),
            responses_debug: Some(responses_debug),
        },
    }
}

async fn oauth_passthrough(
    client: &Client,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: ReqwestBody,
    handshake_timeout: Duration,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
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
                responses_debug: None,
            };
        }
    };
    let mut builder = client
        .request(method, upstream_url)
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental");
    builder = attach_account_header(builder, chatgpt_account_id);
    let (builder, _) = copy_forwardable_headers(builder, headers, &[]);
    let upstream = match timeout(handshake_timeout, builder.body(body).send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to contact oauth codex upstream: {err}"),
                    "oauth_upstream_unavailable",
                ),
                responses_debug: None,
            };
        }
        Err(_) => {
            return OauthUpstreamResponse {
                response: error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!(
                        "oauth codex upstream handshake timed out after {}ms",
                        handshake_timeout.as_millis()
                    ),
                    "oauth_upstream_handshake_timeout",
                ),
                responses_debug: None,
            };
        }
    };
    OauthUpstreamResponse {
        response: reqwest_response_to_axum_response(upstream),
        responses_debug: None,
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
) -> (reqwest::RequestBuilder, OauthForwardedHeaderSummary) {
    let connection_scoped = crate::connection_scoped_header_names(headers);
    let mut forwarded_names = BTreeSet::new();
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
        forwarded_names.insert(name.as_str().to_ascii_lowercase());
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

fn prepare_responses_request_body(body: &[u8]) -> Result<PreparedResponsesRequestBody> {
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
    rewrite.applied = rewrite.added_instructions
        || rewrite.added_store
        || rewrite.forced_stream_true
        || rewrite.removed_max_output_tokens;
    Ok(PreparedResponsesRequestBody {
        wants_stream,
        body: serde_json::to_vec(&value)?,
        rewrite,
    })
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
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        if let Some(message) = value
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
mod tests {
    use super::*;

    #[test]
    fn transform_models_payload_maps_codex_catalog_to_openai_shape() {
        let payload = br#"{"models":[{"slug":"gpt-5.4"},{"slug":"gpt-5.3-codex"}]}"#;
        let value = transform_models_payload(payload).expect("transform models payload");
        assert_eq!(value["object"], "list");
        assert_eq!(value["data"][0]["id"], "gpt-5.4");
        assert_eq!(value["data"][1]["id"], "gpt-5.3-codex");
    }

    #[test]
    fn extract_completed_response_from_sse_returns_completed_response() {
        let payload = b"event: response.created\n\ndata: {\"type\":\"response.created\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"status\":\"completed\"}}\n\n";
        let value =
            extract_completed_response_from_sse(payload).expect("extract completed response");
        assert_eq!(value["id"], "resp_123");
        assert_eq!(value["status"], "completed");
    }

    #[tokio::test]
    async fn oauth_codex_upstream_base_url_uses_test_override() {
        let _guard = TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK.lock().await;
        let override_url =
            Url::parse("http://127.0.0.1:43123/backend-api/codex").expect("valid override url");
        set_test_oauth_codex_upstream_base_url(override_url.clone()).await;
        assert_eq!(
            oauth_codex_upstream_base_url()
                .expect("oauth codex upstream base url")
                .as_str(),
            override_url.as_str()
        );
        reset_test_oauth_codex_upstream_base_url().await;
    }

    #[test]
    fn unsupported_oauth_route_returns_explicit_error() {
        assert!(!is_supported_oauth_passthrough_route("/v1/embeddings"));
        assert!(is_supported_oauth_passthrough_route(
            "/v1/responses/compact"
        ));
        assert!(is_supported_oauth_passthrough_route("/v1/chat/completions"));
    }

    #[test]
    fn attach_account_header_accepts_uuid_style_account_ids() {
        let request = attach_account_header(
            Client::new().get("https://example.com"),
            Some(" 02355c9d-fb23-4517-a96d-35e5f6758e9e "),
        )
        .build()
        .expect("build request");

        assert_eq!(
            request
                .headers()
                .get("ChatGPT-Account-Id")
                .and_then(|value| value.to_str().ok()),
            Some("02355c9d-fb23-4517-a96d-35e5f6758e9e")
        );
    }

    #[test]
    fn attach_account_header_skips_blank_account_ids() {
        let request = attach_account_header(Client::new().get("https://example.com"), Some("   "))
            .build()
            .expect("build request");

        assert!(request.headers().get("ChatGPT-Account-Id").is_none());
    }

    #[test]
    fn prepare_responses_request_body_preserves_previous_response_id() {
        let prepared = prepare_responses_request_body(
            br#"{"model":"gpt-5.4","stream":false,"max_output_tokens":256,"previous_response_id":"resp_prev_001"}"#,
        )
        .expect("rewrite responses request");

        assert!(!prepared.wants_stream);
        let payload: Value = serde_json::from_slice(&prepared.body).expect("decode rewritten body");
        assert_eq!(payload["previous_response_id"], "resp_prev_001");
        assert_eq!(payload["instructions"], "");
        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], true);
        assert!(payload.get("max_output_tokens").is_none());
        assert_eq!(
            prepared.rewrite,
            OauthResponsesRewriteSummary {
                applied: true,
                added_instructions: true,
                added_store: true,
                forced_stream_true: true,
                removed_max_output_tokens: true,
            }
        );
    }

    #[test]
    fn copy_forwardable_headers_keeps_prompt_cache_headers_but_strips_sticky_headers() {
        let client = Client::new();
        let headers = HeaderMap::from_iter([
            (
                header::HeaderName::from_static("x-prompt-cache-key"),
                "prompt-cache-alpha".parse().expect("x prompt cache value"),
            ),
            (
                header::HeaderName::from_static("x-openai-prompt-cache-key"),
                "prompt-cache-beta"
                    .parse()
                    .expect("openai prompt cache value"),
            ),
            (
                header::HeaderName::from_static("x-client-trace-id"),
                "trace-123".parse().expect("client trace id"),
            ),
            (
                header::HeaderName::from_static("x-sticky-key"),
                "sticky-should-not-forward".parse().expect("sticky key"),
            ),
        ]);

        let (builder, summary) =
            copy_forwardable_headers(client.get("https://example.com"), &headers, &[]);
        let request = builder.build().expect("build request");

        assert_eq!(
            request
                .headers()
                .get("x-prompt-cache-key")
                .and_then(|value| value.to_str().ok()),
            Some("prompt-cache-alpha")
        );
        assert_eq!(
            request
                .headers()
                .get("x-openai-prompt-cache-key")
                .and_then(|value| value.to_str().ok()),
            Some("prompt-cache-beta")
        );
        assert_eq!(
            request
                .headers()
                .get("x-client-trace-id")
                .and_then(|value| value.to_str().ok()),
            Some("trace-123")
        );
        assert!(request.headers().get("x-sticky-key").is_none());
        assert!(summary.prompt_cache_header_forwarded);
        assert_eq!(
            summary.names,
            vec![
                "x-client-trace-id".to_string(),
                "x-openai-prompt-cache-key".to_string(),
                "x-prompt-cache-key".to_string()
            ]
        );
    }

    #[test]
    fn oauth_responses_request_overrides_security_headers_after_forwarding() {
        let client = Client::new();
        let headers = HeaderMap::from_iter([
            (
                header::AUTHORIZATION,
                "Bearer client-token".parse().expect("authorization"),
            ),
            (
                header::CONTENT_TYPE,
                "application/custom+json".parse().expect("content type"),
            ),
            (
                header::CONTENT_LENGTH,
                "999".parse().expect("content length"),
            ),
            (
                header::HeaderName::from_static("chatgpt-account-id"),
                "client-account".parse().expect("chatgpt account id"),
            ),
            (
                header::HeaderName::from_static("x-openai-prompt-cache-key"),
                "prompt-cache-gamma".parse().expect("prompt cache key"),
            ),
        ]);

        let builder = client.post("https://example.com");
        let (builder, _) =
            copy_forwardable_headers(builder, &headers, OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES);
        let request = attach_account_header(
            builder
                .bearer_auth("oauth-upstream-token")
                .header(header::CONTENT_TYPE, "application/json")
                .header("OpenAI-Beta", "responses=experimental"),
            Some("server-account"),
        )
        .build()
        .expect("build oauth responses request");

        assert_eq!(
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer oauth-upstream-token")
        );
        assert_eq!(
            request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        assert!(request.headers().get(header::CONTENT_LENGTH).is_none());
        assert_eq!(
            request
                .headers()
                .get("ChatGPT-Account-Id")
                .and_then(|value| value.to_str().ok()),
            Some("server-account")
        );
        assert_eq!(
            request
                .headers()
                .get("x-openai-prompt-cache-key")
                .and_then(|value| value.to_str().ok()),
            Some("prompt-cache-gamma")
        );
    }
}
