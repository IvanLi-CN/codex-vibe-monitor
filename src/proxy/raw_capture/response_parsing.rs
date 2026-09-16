use super::*;

pub(crate) fn deflate_stream_uses_zlib_wrapper(header: &[u8]) -> bool {
    if header.len() < 2 {
        return true;
    }

    let cmf = header[0];
    let flg = header[1];
    let method = cmf & 0x0f;
    let window_bits = cmf >> 4;
    let header_word = (u16::from(cmf) << 8) | u16::from(flg);
    method == 8 && window_bits <= 7 && header_word % 31 == 0
}

#[allow(dead_code)]
pub(crate) fn wrap_decoded_response_reader(
    mut reader: Box<dyn Read + Send>,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let encodings = parse_content_encodings(content_encoding);
    for encoding in encodings.iter().rev() {
        reader = match encoding.as_str() {
            "identity" => reader,
            "gzip" | "x-gzip" => Box::new(GzDecoder::new(reader)),
            "br" => Box::new(BrotliDecompressor::new(reader, 4096)),
            "deflate" => {
                let mut buffered = io::BufReader::new(reader);
                let header = buffered.fill_buf().map_err(|err| err.to_string())?;
                if deflate_stream_uses_zlib_wrapper(header) {
                    Box::new(ZlibDecoder::new(buffered))
                } else {
                    Box::new(DeflateDecoder::new(buffered))
                }
            }
            other => return Err(format!("unsupported_content_encoding:{other}")),
        };
    }
    Ok(reader)
}

#[allow(dead_code)]
pub(crate) fn open_decoded_response_reader(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let file = fs::File::open(path).map_err(|err| err.to_string())?;
    wrap_decoded_response_reader(Box::new(file), content_encoding)
}

#[allow(dead_code)]
pub(crate) fn parse_nonstream_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take((BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    if decoded.len() > BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES {
        decoded.truncate(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES);
        let mut response_info = parse_target_response_payload(target, &decoded, false, None);
        merge_response_capture_reason(
            &mut response_info,
            PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
        );
        return Ok(response_info);
    }
    Ok(parse_target_response_payload(target, &decoded, false, None))
}

#[allow(dead_code)]
pub(crate) fn parse_target_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    if is_stream_hint && target != ProxyCaptureTarget::StandaloneSearch {
        let reader = open_decoded_response_reader(path, content_encoding)?;
        parse_stream_response_payload_from_reader(reader).map_err(|err| err.to_string())
    } else {
        parse_nonstream_response_payload_from_raw_file(target, path, content_encoding)
    }
}

#[allow(dead_code)]
pub(crate) fn parse_target_response_payload_from_capture(
    target: ProxyCaptureTarget,
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if let Some(path) = resp_raw.path.as_deref() {
        let path = PathBuf::from(path);
        match parse_target_response_payload_from_raw_file(
            target,
            &path,
            is_stream_hint,
            content_encoding,
        ) {
            Ok(response_info) => response_info,
            Err(reason) => {
                let mut response_info = parse_target_response_payload(
                    target,
                    preview_bytes,
                    is_stream_hint,
                    content_encoding,
                );
                merge_response_capture_reason(&mut response_info, reason);
                response_info
            }
        }
    } else {
        parse_target_response_payload(target, preview_bytes, is_stream_hint, content_encoding)
    }
}

pub(crate) fn summarize_pool_upstream_http_failure(
    status: StatusCode,
    upstream_request_id_header: Option<&str>,
    bytes: &[u8],
) -> (Option<String>, Option<String>, Option<String>, String) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        let detail = summarize_plaintext_upstream_error(bytes);
        let message = detail.as_deref().map_or_else(
            || format!("pool upstream responded with {}", status.as_u16()),
            |detail| {
                format!(
                    "pool upstream responded with {}: {}",
                    status.as_u16(),
                    detail
                )
            },
        );
        return (
            None,
            detail,
            upstream_request_id_header.map(|value| value.to_string()),
            message,
        );
    };
    let upstream_error_code = extract_upstream_error_code(&value);
    let upstream_error_message = extract_upstream_error_message(&value);
    let upstream_request_id = upstream_request_id_header
        .map(|value| value.to_string())
        .or_else(|| extract_upstream_request_id(&value));

    let detail = upstream_error_message
        .as_deref()
        .or_else(|| value.get("message").and_then(|entry| entry.as_str()))
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.chars().take(240).collect::<String>());

    let message = if let Some(detail) = detail {
        format!(
            "pool upstream responded with {}: {}",
            status.as_u16(),
            detail
        )
    } else {
        format!("pool upstream responded with {}", status.as_u16())
    };

    (
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        message,
    )
}

pub(crate) struct NormalizedPoolFailureRecord {
    pub(crate) attempt_status: &'static str,
    pub(crate) upstream_http_status: Option<StatusCode>,
    pub(crate) downstream_http_status: Option<StatusCode>,
    pub(crate) canonical_error_message: String,
    pub(crate) downstream_error_message: Option<String>,
}

pub(crate) fn default_oauth_transport_failure_message(failure_kind: &'static str) -> &'static str {
    match failure_kind {
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM => "failed to contact oauth codex upstream",
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT => "oauth codex upstream handshake timed out",
        PROXY_FAILURE_UPSTREAM_STREAM_ERROR => "oauth codex upstream stream error",
        _ => "oauth codex upstream transport failure",
    }
}

pub(crate) fn normalize_pool_upstream_failure_record(
    status: StatusCode,
    oauth_transport_failure_kind: Option<&'static str>,
    message: &str,
    upstream_error_message: Option<&str>,
) -> NormalizedPoolFailureRecord {
    if let Some(failure_kind) = oauth_transport_failure_kind {
        let wrapped_prefix = format!("pool upstream responded with {}:", status.as_u16());
        let canonical_error_message = upstream_error_message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                message
                    .strip_prefix(&wrapped_prefix)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| default_oauth_transport_failure_message(failure_kind))
            .to_string();
        return NormalizedPoolFailureRecord {
            attempt_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            upstream_http_status: None,
            downstream_http_status: Some(status),
            canonical_error_message,
            downstream_error_message: Some(message.to_string()),
        };
    }

    NormalizedPoolFailureRecord {
        attempt_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        upstream_http_status: Some(status),
        downstream_http_status: None,
        canonical_error_message: message.to_string(),
        downstream_error_message: None,
    }
}

pub(crate) async fn estimate_proxy_cost_from_shared_catalog(
    catalog: &Arc<RwLock<PricingCatalog>>,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<f64>, bool, Option<String>) {
    let guard = catalog.read().await;
    estimate_proxy_cost(&guard, model, usage, billing_service_tier, pricing_mode)
}

pub(crate) async fn estimate_proxy_cost_breakdown_from_shared_catalog(
    catalog: &Arc<RwLock<PricingCatalog>>,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<ProxyCostBreakdown>, bool, Option<String>) {
    let guard = catalog.read().await;
    estimate_proxy_cost_breakdown(&guard, model, usage, billing_service_tier, pricing_mode)
}

pub(crate) fn has_billable_usage(usage: &ParsedUsage) -> bool {
    usage.input_tokens.unwrap_or(0).max(0) > 0
        || usage.output_tokens.unwrap_or(0).max(0) > 0
        || usage.cache_input_tokens.unwrap_or(0).max(0) > 0
        || usage.reasoning_tokens.unwrap_or(0).max(0) > 0
}

pub(crate) fn resolve_pricing_for_model<'a>(
    catalog: &'a PricingCatalog,
    model: &str,
) -> Option<&'a ModelPricing> {
    if let Some(pricing) = catalog.models.get(model) {
        return Some(pricing);
    }
    dated_model_alias_base(model).and_then(|base| catalog.models.get(base))
}

pub(crate) fn dated_model_alias_base(model: &str) -> Option<&str> {
    const DATED_SUFFIX_LEN: usize = 11; // -YYYY-MM-DD
    if model.len() <= DATED_SUFFIX_LEN {
        return None;
    }
    let suffix = &model.as_bytes()[model.len() - DATED_SUFFIX_LEN..];
    let is_dated_suffix = suffix[0] == b'-'
        && suffix[1].is_ascii_digit()
        && suffix[2].is_ascii_digit()
        && suffix[3].is_ascii_digit()
        && suffix[4].is_ascii_digit()
        && suffix[5] == b'-'
        && suffix[6].is_ascii_digit()
        && suffix[7].is_ascii_digit()
        && suffix[8] == b'-'
        && suffix[9].is_ascii_digit()
        && suffix[10].is_ascii_digit();
    if !is_dated_suffix {
        return None;
    }
    let base = &model[..model.len() - DATED_SUFFIX_LEN];
    if base.is_empty() { None } else { Some(base) }
}

pub(crate) fn is_gpt_5_4_long_context_surcharge_model(model: &str) -> bool {
    let base = dated_model_alias_base(model).unwrap_or(model);
    matches!(base, "gpt-5.4" | "gpt-5.4-pro")
}

pub(crate) fn proxy_price_version(catalog_version: &str, pricing_mode: ProxyPricingMode) -> String {
    format!("{catalog_version}{}", pricing_mode.price_version_suffix())
}

pub(crate) fn pricing_backfill_attempt_version(catalog: &PricingCatalog) -> String {
    fn mix_fvn1a(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }

    let mut hash = 0xcbf29ce484222325_u64;
    mix_fvn1a(&mut hash, COST_BACKFILL_ALGO_VERSION.as_bytes());
    mix_fvn1a(&mut hash, &[0xfc]);
    mix_fvn1a(&mut hash, catalog.version.as_bytes());
    mix_fvn1a(&mut hash, &[0xff]);
    mix_fvn1a(&mut hash, API_KEYS_BILLING_ACCOUNT_KIND.as_bytes());
    mix_fvn1a(&mut hash, &[0xfb]);
    mix_fvn1a(&mut hash, REQUESTED_TIER_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xfa]);
    mix_fvn1a(&mut hash, RESPONSE_TIER_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xf9]);
    mix_fvn1a(&mut hash, EXPLICIT_BILLING_PRICE_VERSION_SUFFIX.as_bytes());
    mix_fvn1a(&mut hash, &[0xf8]);

    let mut models = catalog.models.iter().collect::<Vec<_>>();
    models.sort_by_key(|(a, _)| *a);
    for (model, pricing) in models {
        mix_fvn1a(&mut hash, model.as_bytes());
        mix_fvn1a(&mut hash, &[0xfe]);
        mix_fvn1a(&mut hash, &pricing.input_per_1m.to_bits().to_le_bytes());
        mix_fvn1a(&mut hash, &pricing.output_per_1m.to_bits().to_le_bytes());

        match pricing.effective_cache_read_per_1m() {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        match pricing.cache_write_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        match pricing.reasoning_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        mix_fvn1a(&mut hash, &[0xfd]);
    }

    format!("{}@{:016x}", catalog.version, hash)
}

pub(crate) fn estimate_proxy_cost(
    catalog: &PricingCatalog,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<f64>, bool, Option<String>) {
    let (breakdown, estimated, price_version) =
        estimate_proxy_cost_breakdown(catalog, model, usage, billing_service_tier, pricing_mode);
    (
        breakdown.map(ProxyCostBreakdown::total),
        estimated,
        price_version,
    )
}

pub(crate) fn estimate_proxy_cost_breakdown(
    catalog: &PricingCatalog,
    model: Option<&str>,
    usage: &ParsedUsage,
    billing_service_tier: Option<&str>,
    pricing_mode: ProxyPricingMode,
) -> (Option<ProxyCostBreakdown>, bool, Option<String>) {
    let price_version = Some(proxy_price_version(&catalog.version, pricing_mode));
    let Some(model) = model else {
        return (None, false, price_version);
    };
    let Some(pricing) = resolve_pricing_for_model(catalog, model) else {
        return (None, false, price_version);
    };
    let input_tokens = usage.input_tokens.unwrap_or(0).max(0);
    let output_tokens = usage.output_tokens.unwrap_or(0).max(0) as f64;
    let cache_input_tokens = usage.cache_input_tokens.unwrap_or(0).max(0);
    let reasoning_tokens = usage.reasoning_tokens.unwrap_or(0).max(0) as f64;
    if !has_billable_usage(usage) {
        return (None, false, price_version);
    }

    let apply_long_context_surcharge = is_gpt_5_4_long_context_surcharge_model(model)
        && input_tokens > GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS;
    let apply_priority_billing_multiplier = billing_service_tier
        .and_then(normalize_service_tier)
        .as_deref()
        .is_some_and(|tier| tier == PRIORITY_SERVICE_TIER);

    let cache_read_price = pricing.effective_cache_read_per_1m();
    let billable_cache_tokens = if cache_read_price.is_some() {
        cache_input_tokens
    } else {
        0
    };
    let non_cached_input_tokens = input_tokens.saturating_sub(billable_cache_tokens);

    let mut breakdown = if pricing.has_explicit_cache_pricing_split() {
        let cache_write_price = pricing
            .cache_write_per_1m
            .expect("explicit cache split requires write pricing");
        ProxyCostBreakdown {
            cache_write: (non_cached_input_tokens as f64 / 1_000_000.0) * cache_write_price,
            cache_read: cache_read_price
                .map(|cache_price| (billable_cache_tokens as f64 / 1_000_000.0) * cache_price)
                .unwrap_or(0.0),
            ..ProxyCostBreakdown::default()
        }
    } else {
        ProxyCostBreakdown {
            input: (non_cached_input_tokens as f64 / 1_000_000.0) * pricing.input_per_1m,
            cache_read: cache_read_price
                .map(|cache_price| (billable_cache_tokens as f64 / 1_000_000.0) * cache_price)
                .unwrap_or(0.0),
            ..ProxyCostBreakdown::default()
        }
    };
    breakdown.output = (output_tokens / 1_000_000.0) * pricing.output_per_1m;
    breakdown.reasoning = pricing
        .reasoning_per_1m
        .map(|reasoning_price| (reasoning_tokens / 1_000_000.0) * reasoning_price)
        .unwrap_or(0.0);

    if apply_long_context_surcharge {
        breakdown.input *= 2.0;
        breakdown.cache_write *= 2.0;
        breakdown.cache_read *= 2.0;
        breakdown.output *= 1.5;
        breakdown.reasoning *= 1.5;
    }

    if apply_priority_billing_multiplier {
        breakdown.input *= 2.0;
        breakdown.cache_write *= 2.0;
        breakdown.cache_read *= 2.0;
        breakdown.output *= 2.0;
        breakdown.reasoning *= 2.0;
    }

    (Some(breakdown), true, price_version)
}
