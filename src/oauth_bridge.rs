use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Form, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use futures_util::TryStreamExt;
use rand::{RngCore, rngs::OsRng};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

pub(crate) const OAUTH_BRIDGE_SERVICE_HOST: &str = "ai-openai-oauth-bridge";
pub(crate) const OAUTH_BRIDGE_PORT: u16 = 3000;
const OAUTH_BRIDGE_OPENAI_BASE_URL: &str = "http://ai-openai-oauth-bridge:3000/openai";
const OAUTH_BRIDGE_REGISTER_URL: &str =
    "http://ai-openai-oauth-bridge:3000/internal/token/register";
const OAUTH_BRIDGE_UPSTREAM_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const OAUTH_BRIDGE_MODELS_CLIENT_VERSION: &str = "0.111.0";
const OAUTH_BRIDGE_DEFAULT_TOKEN_TTL_SECS: i64 = 45 * 60;

#[cfg(test)]
use once_cell::sync::Lazy;
#[cfg(test)]
use std::sync::Mutex as StdMutex;

#[cfg(test)]
static TEST_FIXED_ENDPOINTS: Lazy<StdMutex<Option<FixedEndpointsOverride>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg(test)]
pub(crate) static TEST_FIXED_ENDPOINTS_LOCK: Lazy<tokio::sync::Mutex<()>> =
    Lazy::new(|| tokio::sync::Mutex::new(()));

#[cfg(test)]
#[derive(Clone)]
struct FixedEndpointsOverride {
    register_url: Url,
    openai_base_url: Url,
}

#[derive(Debug, Clone)]
pub(crate) struct OauthBridgeRegisterResult {
    pub(crate) token_key: String,
    pub(crate) expire_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct RegisteredToken {
    account_id: String,
    access_token: String,
    expire_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct OauthBridgeState {
    http_client: Client,
    registrations: Arc<Mutex<HashMap<String, RegisteredToken>>>,
}

#[derive(Debug, Deserialize)]
struct OauthBridgeRegisterForm {
    account_id: String,
    access_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OauthBridgeRegisterResponse {
    token_key: String,
    expire_at: i64,
}

pub(crate) fn fixed_oauth_bridge_openai_base_url() -> Result<Url> {
    #[cfg(test)]
    if let Some(override_value) = TEST_FIXED_ENDPOINTS
        .lock()
        .expect("lock test fixed endpoints")
        .as_ref()
        .cloned()
    {
        return Ok(override_value.openai_base_url);
    }

    Url::parse(OAUTH_BRIDGE_OPENAI_BASE_URL)
        .context("failed to parse fixed OAuth bridge openai base url")
}

pub(crate) fn fixed_oauth_bridge_register_url() -> Result<Url> {
    #[cfg(test)]
    if let Some(override_value) = TEST_FIXED_ENDPOINTS
        .lock()
        .expect("lock test fixed endpoints")
        .as_ref()
        .cloned()
    {
        return Ok(override_value.register_url);
    }

    Url::parse(OAUTH_BRIDGE_REGISTER_URL).context("failed to parse fixed OAuth bridge register url")
}

#[cfg(test)]
pub(crate) async fn set_test_oauth_bridge_fixed_endpoints(register_url: Url, openai_base_url: Url) {
    *TEST_FIXED_ENDPOINTS
        .lock()
        .expect("lock test fixed endpoints") = Some(FixedEndpointsOverride {
        register_url,
        openai_base_url,
    });
}

#[cfg(test)]
pub(crate) async fn reset_test_oauth_bridge_fixed_endpoints() {
    *TEST_FIXED_ENDPOINTS
        .lock()
        .expect("lock test fixed endpoints") = None;
}

pub(crate) async fn register_oauth_bridge_access_token(
    client: &Client,
    account_id: &str,
    access_token: &str,
) -> Result<OauthBridgeRegisterResult> {
    let url = fixed_oauth_bridge_register_url()?;
    let response = client
        .post(url)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .form(&[("account_id", account_id), ("access_token", access_token)])
        .send()
        .await
        .context("failed to contact oauth bridge token register endpoint")?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read oauth bridge register response")?;
    if !status.is_success() {
        let detail = summarize_error_detail(&body)
            .unwrap_or_else(|| format!("oauth bridge responded with {}", status.as_u16()));
        bail!("oauth bridge token exchange failed: {detail}");
    }
    let payload: OauthBridgeRegisterResponse =
        serde_json::from_slice(&body).context("failed to decode oauth bridge register response")?;
    Ok(OauthBridgeRegisterResult {
        token_key: payload.token_key,
        expire_at: Utc.timestamp_opt(payload.expire_at, 0).single(),
    })
}

pub(crate) fn oauth_bridge_router(http_client: Client) -> Router {
    let state = OauthBridgeState {
        http_client,
        registrations: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/health", get(oauth_bridge_health))
        .route(
            "/internal/token/register",
            post(oauth_bridge_register_token),
        )
        .route("/openai/v1/models", get(oauth_bridge_models))
        .route("/openai/v1/responses", post(oauth_bridge_responses))
        .route(
            "/openai/v1/responses/compact",
            any(oauth_bridge_passthrough),
        )
        .route("/openai/v1/chat/completions", any(oauth_bridge_passthrough))
        .with_state(state)
}

pub(crate) async fn run_fixed_oauth_bridge_server() -> Result<()> {
    let http_client = Client::builder()
        .user_agent("codex-vibe-monitor-oauth-bridge/0.2.0")
        .build()
        .context("failed to construct oauth bridge http client")?;
    let app = oauth_bridge_router(http_client);
    let bind = SocketAddr::from(([0, 0, 0, 0], OAUTH_BRIDGE_PORT));
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .context("failed to bind fixed oauth bridge listener")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("oauth bridge server exited unexpectedly")
}

async fn oauth_bridge_health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn oauth_bridge_register_token(
    State(state): State<OauthBridgeState>,
    Form(form): Form<OauthBridgeRegisterForm>,
) -> Result<Json<OauthBridgeRegisterResponse>, (StatusCode, String)> {
    let account_id = form.account_id.trim();
    let access_token = form.access_token.trim();
    if account_id.is_empty() || access_token.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "account_id and access_token are required".to_string(),
        ));
    }
    let expire_at = extract_access_token_exp(access_token).unwrap_or_else(|| {
        Utc::now() + ChronoDuration::seconds(OAUTH_BRIDGE_DEFAULT_TOKEN_TTL_SECS)
    });
    let token_key = random_hex(24).map_err(internal_error_tuple)?;
    state.registrations.lock().await.insert(
        token_key.clone(),
        RegisteredToken {
            account_id: account_id.to_string(),
            access_token: access_token.to_string(),
            expire_at,
        },
    );
    Ok(Json(OauthBridgeRegisterResponse {
        token_key,
        expire_at: expire_at.timestamp(),
    }))
}

async fn oauth_bridge_models(
    State(state): State<OauthBridgeState>,
    headers: HeaderMap,
) -> Response {
    let registered = match authorize_bridge_token(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let upstream_url = match Url::parse(&format!(
        "{}/models?client_version={}",
        OAUTH_BRIDGE_UPSTREAM_BASE_URL, OAUTH_BRIDGE_MODELS_CLIENT_VERSION
    )) {
        Ok(url) => url,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid bridge models url: {err}"),
                "server_error",
            );
        }
    };
    let request = state
        .http_client
        .get(upstream_url)
        .bearer_auth(&registered.access_token)
        .header("OpenAI-Beta", "responses=experimental");
    let request = attach_account_header(request, &registered.account_id);
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth bridge upstream request failed: {err}"),
                "oauth_bridge_upstream_unavailable",
            );
        }
    };
    let status = upstream.status();
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth bridge failed to read upstream models response: {err}"),
                "oauth_bridge_upstream_read_failed",
            );
        }
    };
    if !status.is_success() {
        return json_or_plain_error_response(
            status,
            &bytes,
            "oauth_bridge_upstream_rejected_request",
        );
    }
    match transform_models_payload(&bytes) {
        Ok(value) => (status, Json(value)).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_GATEWAY,
            &format!("oauth bridge returned malformed models payload: {err}"),
            "oauth_bridge_upstream_invalid_models",
        ),
    }
}

async fn oauth_bridge_responses(
    State(state): State<OauthBridgeState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let registered = match authorize_bridge_token(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
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
    let upstream_url = match Url::parse(&format!("{OAUTH_BRIDGE_UPSTREAM_BASE_URL}/responses")) {
        Ok(url) => url,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid bridge responses url: {err}"),
                "server_error",
            );
        }
    };
    let request = state
        .http_client
        .post(upstream_url)
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(&registered.access_token)
        .header("OpenAI-Beta", "responses=experimental")
        .body(upstream_body);
    let request = attach_account_header(request, &registered.account_id);
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth bridge upstream request failed: {err}"),
                "oauth_bridge_upstream_unavailable",
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
                    &format!("oauth bridge failed to read upstream error response: {err}"),
                    "oauth_bridge_upstream_read_failed",
                );
            }
        };
        return json_or_plain_error_response(
            status,
            &bytes,
            "oauth_bridge_upstream_rejected_request",
        );
    }
    if wants_stream {
        return reqwest_response_to_axum_response(upstream);
    }
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth bridge failed to read upstream responses stream: {err}"),
                "oauth_bridge_upstream_read_failed",
            );
        }
    };
    match extract_completed_response_from_sse(&bytes) {
        Ok(response_value) => (StatusCode::OK, Json(response_value)).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_GATEWAY,
            &format!("oauth bridge failed to decode Codex response stream: {err}"),
            "oauth_bridge_upstream_invalid_response",
        ),
    }
}

async fn oauth_bridge_passthrough(
    State(state): State<OauthBridgeState>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    let registered = match authorize_bridge_token(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let method = request.method().clone();
    let uri = request.uri().clone();
    let body = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("failed to read oauth bridge request body: {err}"),
                "invalid_request_error",
            );
        }
    };
    let suffix = uri.path().trim_start_matches("/openai/v1");
    let upstream_url = match Url::parse(&format!("{}{}", OAUTH_BRIDGE_UPSTREAM_BASE_URL, suffix)) {
        Ok(url) => url,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("invalid bridge upstream url: {err}"),
                "server_error",
            );
        }
    };
    let mut builder = state
        .http_client
        .request(method, upstream_url)
        .bearer_auth(&registered.access_token)
        .header("OpenAI-Beta", "responses=experimental");
    builder = attach_account_header(builder, &registered.account_id);
    builder = copy_forwardable_headers(builder, &headers);
    let upstream = match builder.body(body).send().await {
        Ok(response) => response,
        Err(err) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("oauth bridge upstream request failed: {err}"),
                "oauth_bridge_upstream_unavailable",
            );
        }
    };
    reqwest_response_to_axum_response(upstream)
}

async fn authorize_bridge_token(
    state: &OauthBridgeState,
    headers: &HeaderMap,
) -> Result<RegisteredToken, Response> {
    let Some(token_key) = extract_bearer_token(headers) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "oauth bridge requires a bearer token",
            "invalid_api_key",
        ));
    };
    let Some(registered) = state.registrations.lock().await.get(&token_key).cloned() else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "oauth bridge token is unknown or was not registered",
            "invalid_api_key",
        ));
    };
    if registered.expire_at <= Utc::now() {
        state.registrations.lock().await.remove(&token_key);
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "oauth bridge token expired; register again",
            "token_expired",
        ));
    }
    Ok(registered)
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
    account_id: &str,
) -> reqwest::RequestBuilder {
    if account_id.starts_with("org_") {
        builder.header("ChatGPT-Account-Id", account_id)
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
                &format!("oauth bridge failed to stream upstream response: {err}"),
                "oauth_bridge_stream_error",
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
                "owned_by": "oauth-bridge",
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "object": "list", "data": data }))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let authorization = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn extract_access_token_exp(access_token: &str) -> Option<DateTime<Utc>> {
    let payload = access_token.split('.').nth(1)?;
    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .ok()
        })?;
    let value: Value = serde_json::from_slice(&payload_bytes).ok()?;
    let exp = value.get("exp")?.as_i64()?;
    Utc.timestamp_opt(exp, 0).single()
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
        .unwrap_or_else(|| format!("oauth bridge upstream responded with {}", status.as_u16()));
    error_response(status, &message, code)
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

fn internal_error_tuple(err: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn random_hex(size: usize) -> Result<String> {
    let mut bytes = vec![0u8; size];
    OsRng.fill_bytes(&mut bytes);
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
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
    async fn register_oauth_bridge_access_token_uses_fixed_register_endpoint() {
        let _guard = TEST_FIXED_ENDPOINTS_LOCK.lock().await;
        let app = Router::new().route(
            "/internal/token/register",
            post(|Form(form): Form<OauthBridgeRegisterForm>| async move {
                assert_eq!(form.account_id, "org_test");
                assert_eq!(form.access_token, "access-test");
                Json(OauthBridgeRegisterResponse {
                    token_key: "token-key-1".to_string(),
                    expire_at: 1_900_000_000,
                })
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test bridge register");
        let addr = listener.local_addr().expect("bridge register addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve test bridge register");
        });

        let base = Url::parse(&format!("http://{addr}")).expect("parse test bridge base");
        let register_url = base
            .join("/internal/token/register")
            .expect("join register path");
        let openai_base = base.join("/openai").expect("join openai path");
        set_test_oauth_bridge_fixed_endpoints(register_url, openai_base).await;

        let client = Client::new();
        let result = register_oauth_bridge_access_token(&client, "org_test", "access-test")
            .await
            .expect("register oauth bridge access token");
        assert_eq!(result.token_key, "token-key-1");
        assert_eq!(
            result.expire_at.expect("expire_at").timestamp(),
            1_900_000_000
        );

        reset_test_oauth_bridge_fixed_endpoints().await;
        handle.abort();
    }

    #[tokio::test]
    async fn oauth_bridge_router_rejects_unknown_bearer_token() {
        let app = oauth_bridge_router(Client::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind oauth bridge");
        let addr = listener.local_addr().expect("oauth bridge addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve oauth bridge");
        });

        let response = reqwest::get(format!("http://{addr}/health"))
            .await
            .expect("health response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = Client::new()
            .get(format!("http://{addr}/openai/v1/models"))
            .bearer_auth("missing-token")
            .send()
            .await
            .expect("request bridge models");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        handle.abort();
    }
}
