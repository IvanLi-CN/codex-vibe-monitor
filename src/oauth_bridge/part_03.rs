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
