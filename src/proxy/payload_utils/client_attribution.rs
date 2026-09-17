use super::*;
pub(crate) const PROMPT_CACHE_ATTRIBUTION_TTL: Duration = Duration::from_secs(15 * 60);
pub(crate) const CLIENT_ATTRIBUTION_FINGERPRINT_VERSION: &str = "v1";

pub(crate) static CLIENT_PROMPT_CACHE_ATTRIBUTION: Lazy<
    std::sync::Mutex<HashMap<String, ClientPromptCacheAttributionBucket>>,
> = Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Default)]
pub(crate) struct ClientPromptCacheAttributionContext {
    pub(crate) cache_key: Option<String>,
    pub(crate) fingerprint: Option<String>,
    pub(crate) header_fingerprints: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClientPromptCacheAttributionEntry {
    pub(crate) prompt_cache_key: String,
    pub(crate) sticky_key: Option<String>,
    seen_at: Instant,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ClientPromptCacheAttributionBucket {
    entries: HashMap<String, ClientPromptCacheAttributionEntry>,
}

pub(crate) fn short_sha256_fingerprint(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub(crate) fn normalized_header_component(
    headers: &HeaderMap,
    name: &'static str,
) -> Option<String> {
    header_value_as_str(headers, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn client_prompt_cache_attribution_context_from_headers(
    headers: &HeaderMap,
) -> ClientPromptCacheAttributionContext {
    const STRONG_STABLE_HEADER_NAMES: &[&str] = &["session_id", "x-codex-window-id"];
    const STABLE_HEADER_NAMES: &[&str] = &["session_id", "originator", "x-codex-window-id"];
    const DIAGNOSTIC_HEADER_NAMES: &[&str] = &[
        "session_id",
        "originator",
        "x-codex-window-id",
        "x-codex-installation-id",
        "traceparent",
    ];

    let mut raw_components = BTreeMap::new();
    let mut header_fingerprints = BTreeMap::new();
    for name in DIAGNOSTIC_HEADER_NAMES {
        let Some(value) = normalized_header_component(headers, name) else {
            continue;
        };
        header_fingerprints.insert((*name).to_string(), short_sha256_fingerprint(&value));
    }
    for name in STABLE_HEADER_NAMES {
        let Some(value) = normalized_header_component(headers, name) else {
            continue;
        };
        raw_components.insert((*name).to_string(), value);
    }

    let has_strong_stable_component = STRONG_STABLE_HEADER_NAMES
        .iter()
        .any(|name| raw_components.contains_key(*name));
    if raw_components.is_empty() || !has_strong_stable_component {
        return ClientPromptCacheAttributionContext {
            cache_key: None,
            fingerprint: None,
            header_fingerprints,
        };
    }

    let canonical = raw_components
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let fingerprint = format!(
        "{}:{}",
        CLIENT_ATTRIBUTION_FINGERPRINT_VERSION,
        short_sha256_fingerprint(&canonical)
    );

    ClientPromptCacheAttributionContext {
        cache_key: Some(fingerprint.clone()),
        fingerprint: Some(fingerprint),
        header_fingerprints,
    }
}

pub(crate) fn remember_prompt_cache_attribution(
    context: &ClientPromptCacheAttributionContext,
    prompt_cache_key: &str,
    sticky_key: Option<&str>,
    now: Instant,
) {
    let Some(cache_key) = context.cache_key.as_deref() else {
        return;
    };
    let prompt_cache_key = prompt_cache_key.trim();
    if prompt_cache_key.is_empty() {
        return;
    }
    let sticky_key = sticky_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let mut cache = CLIENT_PROMPT_CACHE_ATTRIBUTION
        .lock()
        .expect("client prompt-cache attribution mutex poisoned");
    cache.retain(|_, bucket| {
        bucket
            .entries
            .retain(|_, entry| now.duration_since(entry.seen_at) <= PROMPT_CACHE_ATTRIBUTION_TTL);
        !bucket.entries.is_empty()
    });
    let bucket = cache.entry(cache_key.to_string()).or_default();
    bucket.entries.insert(
        prompt_cache_key.to_string(),
        ClientPromptCacheAttributionEntry {
            prompt_cache_key: prompt_cache_key.to_string(),
            sticky_key,
            seen_at: now,
        },
    );
}

pub(crate) fn lookup_recent_prompt_cache_attribution(
    context: &ClientPromptCacheAttributionContext,
    now: Instant,
) -> Option<ClientPromptCacheAttributionEntry> {
    let cache_key = context.cache_key.as_deref()?;
    let mut cache = CLIENT_PROMPT_CACHE_ATTRIBUTION
        .lock()
        .expect("client prompt-cache attribution mutex poisoned");
    cache.retain(|_, bucket| {
        bucket
            .entries
            .retain(|_, entry| now.duration_since(entry.seen_at) <= PROMPT_CACHE_ATTRIBUTION_TTL);
        !bucket.entries.is_empty()
    });
    let bucket = cache.get(cache_key)?;
    if bucket.entries.len() != 1 {
        return None;
    }
    bucket.entries.values().next().cloned()
}

#[cfg(test)]
pub(crate) fn clear_prompt_cache_attribution_for_tests() {
    CLIENT_PROMPT_CACHE_ATTRIBUTION
        .lock()
        .expect("client prompt-cache attribution mutex poisoned")
        .clear();
}

pub(crate) fn extract_requester_ip(headers: &HeaderMap, peer_ip: Option<IpAddr>) -> Option<String> {
    if let Some(x_forwarded_for) = header_value_as_str(headers, "x-forwarded-for")
        && let Some(ip) = extract_first_ip_from_x_forwarded_for(x_forwarded_for)
    {
        return Some(ip);
    }

    if let Some(x_real_ip) = header_value_as_str(headers, "x-real-ip")
        && let Some(ip) = extract_ip_from_header_value(x_real_ip)
    {
        return Some(ip);
    }

    if let Some(forwarded) = header_value_as_str(headers, "forwarded")
        && let Some(ip) = extract_ip_from_forwarded_header(forwarded)
    {
        return Some(ip);
    }

    peer_ip.map(|ip| ip.to_string())
}

pub(crate) const REQUEST_CHAIN_METADATA_MAX_BYTES: usize = 512;

#[derive(Debug, Clone, Default)]
pub(crate) struct RequestChainMetadata {
    pub(crate) user_agent: Option<String>,
    pub(crate) x_forwarded_for: Option<String>,
    pub(crate) forwarded: Option<String>,
    pub(crate) x_real_ip: Option<String>,
}

pub(crate) fn truncate_header_value(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }

    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    raw[..end].to_string()
}

pub(crate) fn bounded_request_header_value(
    headers: &HeaderMap,
    name: &'static str,
) -> Option<String> {
    header_value_as_str(headers, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| truncate_header_value(value, REQUEST_CHAIN_METADATA_MAX_BYTES))
}

pub(crate) fn request_chain_metadata_from_headers(headers: &HeaderMap) -> RequestChainMetadata {
    RequestChainMetadata {
        user_agent: bounded_request_header_value(headers, "user-agent"),
        x_forwarded_for: bounded_request_header_value(headers, "x-forwarded-for"),
        forwarded: bounded_request_header_value(headers, "forwarded"),
        x_real_ip: bounded_request_header_value(headers, "x-real-ip"),
    }
}

pub(crate) fn extract_sticky_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-sticky-key",
        "sticky-key",
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_prompt_cache_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_first_ip_from_x_forwarded_for(raw: &str) -> Option<String> {
    let first = raw.split(',').next()?.trim();
    extract_ip_from_header_value(first)
}

pub(crate) fn extract_ip_from_forwarded_header(raw: &str) -> Option<String> {
    for entry in raw.split(',') {
        for segment in entry.split(';') {
            let pair = segment.trim();
            if pair.len() >= 4 && pair[..4].eq_ignore_ascii_case("for=") {
                let value = &pair[4..];
                if let Some(ip) = extract_ip_from_header_value(value) {
                    return Some(ip);
                }
            }
        }
    }
    None
}

pub(crate) fn extract_ip_from_header_value(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_matches('"');
    if normalized.is_empty()
        || normalized.eq_ignore_ascii_case("unknown")
        || normalized.starts_with('_')
    {
        return None;
    }

    if let Some(value) = normalized.strip_prefix("for=") {
        return extract_ip_from_header_value(value);
    }

    if normalized.starts_with('[')
        && let Some(end) = normalized.find(']')
        && let Ok(ip) = normalized[1..end].parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return Some(ip.to_string());
    }

    if let Some((host, port)) = normalized.rsplit_once(':')
        && !host.contains(':')
        && port.parse::<u16>().is_ok()
        && let Ok(ip) = host.parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    None
}

pub(crate) fn is_loopback_authority_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub(crate) fn build_preset_models_payload(enabled_model_ids: &[String]) -> Value {
    let data = enabled_model_ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "owned_by": "proxy",
                "created": 0
            })
        })
        .collect::<Vec<_>>();
    json!({
        "object": "list",
        "data": data
    })
}

pub(crate) fn merge_models_payload_with_upstream(
    upstream_payload: &Value,
    enabled_model_ids: &[String],
) -> Result<Value> {
    let upstream_items = upstream_payload
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("upstream models payload missing data array"))?;
    let mut merged = build_preset_models_payload(enabled_model_ids)
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen_ids: HashSet<String> = enabled_model_ids.iter().cloned().collect();

    for item in upstream_items {
        if let Some(id) = item.get("id").and_then(|v| v.as_str())
            && seen_ids.insert(id.to_string())
        {
            merged.push(item.clone());
        }
    }

    Ok(json!({
        "object": "list",
        "data": merged
    }))
}
