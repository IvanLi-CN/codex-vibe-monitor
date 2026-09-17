use super::*;
pub(crate) fn decode_response_payload_for_usage<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, true)
}

pub(crate) fn decode_response_payload<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
    allow_gzip_magic_fallback: bool,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        if allow_gzip_magic_fallback && response_payload_looks_like_gzip_magic(bytes) {
            return decode_single_content_encoding(bytes, "gzip")
                .map(|decoded| (decoded, None))
                .unwrap_or_else(|err| {
                    (
                        Cow::Borrowed(bytes),
                        Some(format!("response_gzip_decode_error:{err}")),
                    )
                });
        }
        return (Cow::Borrowed(bytes), None);
    }

    let mut encodings = encodings.iter().rev();
    let first_encoding = encodings.next().expect("non-empty encodings checked above");
    let mut decoded = match decode_single_content_encoding(bytes, first_encoding) {
        Ok(next) => next.into_owned(),
        Err(err) => {
            return (
                Cow::Borrowed(bytes),
                Some(format!("{first_encoding}:{err}")),
            );
        }
    };
    for encoding in encodings {
        match decode_single_content_encoding(decoded.as_slice(), encoding) {
            Ok(next) => decoded = next.into_owned(),
            Err(err) => {
                return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}")));
            }
        }
    }
    (Cow::Owned(decoded), None)
}

pub(crate) fn parse_content_encodings(content_encoding: Option<&str>) -> Vec<String> {
    content_encoding
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

pub(crate) fn decode_single_content_encoding<'a>(
    bytes: &'a [u8],
    encoding: &str,
) -> std::result::Result<Cow<'a, [u8]>, String> {
    match encoding {
        "identity" => Ok(Cow::Borrowed(bytes)),
        "gzip" | "x-gzip" => decode_gzip_payload(bytes),
        "br" => decode_brotli_payload(bytes),
        "deflate" => decode_deflate_payload(bytes),
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

pub(crate) fn decode_gzip_payload<'a>(
    bytes: &'a [u8],
) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

pub(crate) fn decode_brotli_payload<'a>(
    bytes: &'a [u8],
) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = BrotliDecompressor::new(bytes, 4096);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

pub(crate) fn decode_deflate_payload<'a>(
    bytes: &'a [u8],
) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut zlib_decoder = ZlibDecoder::new(bytes);
    let mut decoded = Vec::new();
    match zlib_decoder.read_to_end(&mut decoded) {
        Ok(_) => Ok(Cow::Owned(decoded)),
        Err(zlib_err) => {
            let mut raw_decoder = DeflateDecoder::new(bytes);
            let mut raw_decoded = Vec::new();
            raw_decoder
                .read_to_end(&mut raw_decoded)
                .map_err(|raw_err| format!("zlib={zlib_err}; raw={raw_err}"))?;
            Ok(Cow::Owned(raw_decoded))
        }
    }
}

pub(crate) fn response_payload_looks_like_gzip_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

pub(crate) fn extract_model_from_payload(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .pointer("/response/model")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

pub(crate) fn extract_partial_json_string_field(bytes: &[u8], keys: &[&str]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*"((?:\\.|[^"\\])*)""#, regex::escape(key));
        let regex = Regex::new(&pattern).ok()?;
        let captures = regex.captures(text)?;
        let value = captures.get(1)?.as_str();
        serde_json::from_str::<String>(&format!("\"{value}\"")).ok()
    })
}

pub(crate) fn extract_partial_json_model(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["model"])
}

pub(crate) fn extract_partial_json_service_tier(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["service_tier", "serviceTier"])
        .and_then(|value| normalize_service_tier(&value))
}

pub(crate) fn normalize_service_tier(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) const AUTO_SERVICE_TIER: &str = "auto";
pub(crate) const DEFAULT_SERVICE_TIER: &str = "default";
pub(crate) const PRIORITY_SERVICE_TIER: &str = "priority";
pub(crate) const API_KEYS_BILLING_ACCOUNT_KIND: &str = "api_key_codex";
pub(crate) const REQUESTED_TIER_PRICE_VERSION_SUFFIX: &str = "@requested-tier";
pub(crate) const RESPONSE_TIER_PRICE_VERSION_SUFFIX: &str = "@response-tier";
pub(crate) const EXPLICIT_BILLING_PRICE_VERSION_SUFFIX: &str = "@explicit-billing";
pub(crate) const SERVICE_TIER_STREAM_BACKFILL_VERSION: &str = "stream-terminal-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyPricingMode {
    ResponseTier,
    RequestedTier,
    ExplicitBilling,
}

impl ProxyPricingMode {
    pub(crate) fn price_version_suffix(self) -> &'static str {
        match self {
            Self::ResponseTier => RESPONSE_TIER_PRICE_VERSION_SUFFIX,
            Self::RequestedTier => REQUESTED_TIER_PRICE_VERSION_SUFFIX,
            Self::ExplicitBilling => EXPLICIT_BILLING_PRICE_VERSION_SUFFIX,
        }
    }
}

pub(crate) fn normalize_upstream_base_url_host_value(raw: &str) -> Option<String> {
    let host = raw.trim().trim_end_matches('/').to_ascii_lowercase();
    if host.is_empty() || host.contains('/') {
        None
    } else {
        Some(host)
    }
}

pub(crate) fn normalize_upstream_base_url_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| {
            url.host_str()
                .and_then(normalize_upstream_base_url_host_value)
        })
        .or_else(|| normalize_upstream_base_url_host_value(raw))
        .filter(|host| !host.is_empty())
}

pub(crate) fn api_keys_billing_matches_context(upstream_account_kind: Option<&str>) -> bool {
    upstream_account_kind
        .map(str::trim)
        .is_some_and(|kind| kind.eq_ignore_ascii_case(API_KEYS_BILLING_ACCOUNT_KIND))
}

pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode(
    explicit_billing_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
    response_service_tier: Option<&str>,
    upstream_account_kind: Option<&str>,
) -> (Option<String>, ProxyPricingMode) {
    if let Some(explicit_billing_service_tier) =
        explicit_billing_service_tier.and_then(normalize_service_tier)
    {
        return (
            Some(explicit_billing_service_tier),
            ProxyPricingMode::ExplicitBilling,
        );
    }

    let normalized_requested_service_tier = requested_service_tier.and_then(normalize_service_tier);
    let normalized_response_service_tier = response_service_tier.and_then(normalize_service_tier);
    if api_keys_billing_matches_context(upstream_account_kind)
        && normalized_requested_service_tier.is_some()
    {
        return (
            normalized_requested_service_tier,
            ProxyPricingMode::RequestedTier,
        );
    }

    (
        normalized_response_service_tier,
        ProxyPricingMode::ResponseTier,
    )
}

pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
    explicit_billing_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
    response_service_tier: Option<&str>,
    account: Option<&PoolResolvedAccount>,
) -> (Option<String>, ProxyPricingMode) {
    resolve_proxy_billing_service_tier_and_pricing_mode(
        explicit_billing_service_tier,
        requested_service_tier,
        response_service_tier,
        account.map(|entry| entry.kind.as_str()),
    )
}

pub(crate) fn payload_summary_upstream_account_kind(
    account: Option<&PoolResolvedAccount>,
) -> Option<&str> {
    account.map(|entry| entry.kind.as_str())
}

pub(crate) fn payload_summary_upstream_base_url_host(
    account: Option<&PoolResolvedAccount>,
) -> Option<&str> {
    account.and_then(|entry| entry.upstream_base_url.host_str())
}

pub(crate) fn resolve_backfill_upstream_account_kind(
    snapshot_kind: Option<&str>,
    live_kind: Option<&str>,
    allow_live_fallback: bool,
) -> Option<String> {
    if let Some(value) = snapshot_kind
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(value.to_string());
    }

    if !allow_live_fallback {
        return None;
    }

    live_kind
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn allow_live_upstream_account_fallback(raw: Option<i64>) -> bool {
    raw == Some(1)
}

pub(crate) fn resolve_backfill_upstream_base_url_host(
    snapshot_host: Option<&str>,
    live_host: Option<&str>,
    allow_live_fallback: bool,
) -> Option<String> {
    snapshot_host
        .and_then(normalize_upstream_base_url_host)
        .or_else(|| {
            allow_live_fallback
                .then_some(live_host)
                .flatten()
                .and_then(normalize_upstream_base_url_host)
        })
}

pub(crate) fn extract_service_tier_from_payload(value: &Value) -> Option<String> {
    [
        "/service_tier",
        "/serviceTier",
        "/response/service_tier",
        "/response/serviceTier",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(|v| v.as_str()))
    .and_then(normalize_service_tier)
}

pub(crate) fn extract_usage_from_payload(value: &Value) -> Option<ParsedUsage> {
    if let Some(usage) = value.get("usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    if let Some(usage) = value.pointer("/response/usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    None
}

pub(crate) fn parse_usage_value(value: &Value) -> ParsedUsage {
    let input_tokens = value
        .get("input_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("prompt_tokens").and_then(json_value_to_i64));
    let output_tokens = value
        .get("output_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("completion_tokens").and_then(json_value_to_i64));
    let cache_input_tokens = value
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(json_value_to_i64)
        });
    let reasoning_tokens = value
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(json_value_to_i64)
        });

    let mut parsed = ParsedUsage {
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        total_tokens: value.get("total_tokens").and_then(json_value_to_i64),
    };

    if parsed.total_tokens.is_none() {
        parsed.total_tokens = match (parsed.input_tokens, parsed.output_tokens) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        };
    }

    parsed
}

pub(crate) fn json_value_to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return i64::try_from(v).ok();
    }
    value.as_str().and_then(|v| v.parse::<i64>().ok())
}

pub(crate) fn upstream_account_id_from_payload(payload: Option<&str>) -> Option<i64> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value.get("upstreamAccountId").and_then(json_value_to_i64)
}
