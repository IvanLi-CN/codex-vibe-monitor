use anyhow::{Context, Result, bail};
use axum::{
    Json,
    body::{Body, Bytes},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use reqwest::{Body as ReqwestBody, Client, Url};
use serde_json::{Value, json};

#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(test)]
use std::sync::Mutex as StdMutex;

const OAUTH_CODEX_UPSTREAM_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const OAUTH_CODEX_MODELS_CLIENT_VERSION: &str = "0.111.0";

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

pub(crate) async fn send_oauth_upstream_request(
    client: &Client,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: OauthUpstreamRequestBody,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Response {
    match original_uri.path() {
        "/v1/models" => oauth_models(client, access_token, chatgpt_account_id).await,
        "/v1/responses" => {
            oauth_responses(
                client,
                access_token,
                chatgpt_account_id,
                match body {
                    OauthUpstreamRequestBody::Empty => Bytes::new(),
                    OauthUpstreamRequestBody::Bytes(bytes) => bytes,
                    OauthUpstreamRequestBody::Stream(_) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "streamed request bodies are not supported for /v1/responses",
                            "server_error",
                        );
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
                access_token,
                chatgpt_account_id,
            )
            .await
        }
        _ => error_response(
            StatusCode::NOT_FOUND,
            &format!(
                "oauth upstream route is not supported: {}",
                original_uri.path()
            ),
            "oauth_unsupported_route",
        ),
    }
}

fn is_supported_oauth_passthrough_route(path: &str) -> bool {
    matches!(path, "/v1/responses/compact" | "/v1/chat/completions")
}

async fn oauth_models(
    client: &Client,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Response {
    let mut upstream_url = match oauth_codex_upstream_base_url() {
        Ok(url) => url,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid oauth codex models url: {err}"),
                "server_error",
            );
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
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                "oauth_upstream_unavailable",
            );
        }
    };
    let status = upstream.status();
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to read oauth codex models response: {err}"),
                "oauth_upstream_read_failed",
            );
        }
    };
    if !status.is_success() {
        return json_or_plain_error_response(status, &bytes, "oauth_upstream_rejected_request");
    }
    match transform_models_payload(&bytes) {
        Ok(value) => (status, Json(value)).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_GATEWAY,
            &format!("oauth codex returned malformed models payload: {err}"),
            "oauth_upstream_invalid_models",
        ),
    }
}

async fn oauth_responses(
    client: &Client,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    body: Bytes,
) -> Response {
    let (wants_stream, upstream_body) = match prepare_responses_request_body(&body) {
        Ok(value) => value,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                &err.to_string(),
                "invalid_request_error",
            );
        }
    };
    let upstream_url = match build_oauth_upstream_url("/responses", None) {
        Ok(url) => url,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid oauth codex responses url: {err}"),
                "server_error",
            );
        }
    };
    let request = client
        .post(upstream_url)
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental")
        .body(upstream_body);
    let request = attach_account_header(request, chatgpt_account_id);
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                "oauth_upstream_unavailable",
            );
        }
    };
    if !upstream.status().is_success() {
        let status = upstream.status();
        let bytes = match upstream.bytes().await {
            Ok(bytes) => bytes,
            Err(err) => {
                return error_response(
                    StatusCode::BAD_GATEWAY,
                    &format!("failed to read oauth codex error response: {err}"),
                    "oauth_upstream_read_failed",
                );
            }
        };
        return json_or_plain_error_response(status, &bytes, "oauth_upstream_rejected_request");
    }
    if wants_stream {
        return reqwest_response_to_axum_response(upstream);
    }
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to read oauth codex responses stream: {err}"),
                "oauth_upstream_read_failed",
            );
        }
    };
    match extract_completed_response_from_sse(&bytes) {
        Ok(response_value) => (StatusCode::OK, Json(response_value)).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_GATEWAY,
            &format!("failed to decode oauth codex response stream: {err}"),
            "oauth_upstream_invalid_response",
        ),
    }
}

async fn oauth_passthrough(
    client: &Client,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: ReqwestBody,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Response {
    let suffix = original_uri
        .path()
        .strip_prefix("/v1")
        .unwrap_or(original_uri.path());
    let upstream_url = match build_oauth_upstream_url(suffix, original_uri.query()) {
        Ok(url) => url,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid oauth codex upstream url: {err}"),
                "server_error",
            );
        }
    };
    let mut builder = client
        .request(method, upstream_url)
        .bearer_auth(access_token)
        .header("OpenAI-Beta", "responses=experimental");
    builder = attach_account_header(builder, chatgpt_account_id);
    builder = copy_forwardable_headers(builder, headers);
    let upstream = match builder.body(body).send().await {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("failed to contact oauth codex upstream: {err}"),
                "oauth_upstream_unavailable",
            );
        }
    };
    reqwest_response_to_axum_response(upstream)
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
) -> reqwest::RequestBuilder {
    for (name, value) in headers {
        if matches!(
            name.as_str(),
            "authorization" | "host" | "content-length" | "connection"
        ) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
}

fn attach_account_header(
    builder: reqwest::RequestBuilder,
    chatgpt_account_id: Option<&str>,
) -> reqwest::RequestBuilder {
    if chatgpt_account_id.is_some_and(|value| value.starts_with("org_")) {
        builder.header("ChatGPT-Account-Id", chatgpt_account_id.unwrap_or_default())
    } else {
        builder
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

fn prepare_responses_request_body(body: &[u8]) -> Result<(bool, Vec<u8>)> {
    let mut value: Value =
        serde_json::from_slice(body).context("request body must be valid JSON")?;
    let Value::Object(ref mut map) = value else {
        bail!("request body must be a JSON object");
    };
    let wants_stream = map.get("stream").and_then(Value::as_bool).unwrap_or(false);
    if !map.contains_key("instructions") {
        map.insert("instructions".to_string(), Value::String(String::new()));
    }
    if !map.contains_key("store") {
        map.insert("store".to_string(), Value::Bool(false));
    }
    map.insert("stream".to_string(), Value::Bool(true));
    map.remove("max_output_tokens");
    Ok((wants_stream, serde_json::to_vec(&value)?))
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
}
