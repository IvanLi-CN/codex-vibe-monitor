use super::*;
pub(crate) fn parse_cors_allowed_origins_env(name: &str) -> Result<Vec<String>> {
    match env::var(name) {
        Ok(raw) => parse_cors_allowed_origins(&raw),
        Err(env::VarError::NotPresent) => Ok(Vec::new()),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

pub(crate) fn parse_cors_allowed_origins(raw: &str) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for candidate in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let normalized = normalize_cors_origin(candidate)
            .ok_or_else(|| anyhow!("invalid {ENV_CORS_ALLOWED_ORIGINS} entry: {candidate}"))?;
        if seen.insert(normalized.clone()) {
            entries.push(normalized);
        }
    }
    Ok(entries)
}

pub(crate) fn normalize_cors_origin(origin_raw: &str) -> Option<String> {
    let origin = Url::parse(origin_raw).ok()?;
    if !matches!(origin.scheme(), "http" | "https") {
        return None;
    }
    if origin.cannot_be_a_base()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return None;
    }
    if origin.path() != "/" {
        return None;
    }

    let host = origin.host_str()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let scheme = origin.scheme().to_ascii_lowercase();
    let port = origin.port();
    let default_port = default_port_for_scheme(&scheme);

    if port.is_none() || port == default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{}", port?))
    }
}

pub(crate) fn request_public_origin(
    headers: &HeaderMap,
    configured_public_origin: Option<&str>,
) -> Option<String> {
    if let Some(origin) = configured_public_origin {
        return Some(origin.to_string());
    }

    let scheme = request_public_scheme(headers).unwrap_or_else(|| "http".to_string());
    let (host, port) = forwarded_or_host_authority(headers, &scheme)?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let default_port = default_port_for_scheme(&scheme);

    if port.is_none() || port == default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{}", port?))
    }
}

pub(crate) fn request_public_scheme(headers: &HeaderMap) -> Option<String> {
    if let Some(raw) = header_value_as_str(headers, "x-forwarded-proto") {
        let proto = single_forwarded_header_value(raw)?.to_ascii_lowercase();
        return match proto.as_str() {
            "http" | "https" => Some(proto),
            _ => None,
        };
    }

    let forwarded = header_value_as_str(headers, "forwarded")?;
    let proto = forwarded_header_param(forwarded, "proto")?.to_ascii_lowercase();
    match proto.as_str() {
        "http" | "https" => Some(proto),
        _ => None,
    }
}

pub(crate) fn is_models_list_path(path: &str) -> bool {
    path == "/v1/models"
}

// Browser-side CSRF mitigation for settings writes.
//
// This is intentionally not a full authentication mechanism: non-browser clients
// (CLI/automation) may omit Origin and are allowed by policy. The security boundary
// is deployment-level network isolation (trusted gateway only), documented in
// docs/deployment.md.
pub(crate) fn is_same_origin_settings_write(headers: &HeaderMap) -> bool {
    if matches!(
        header_value_as_str(headers, "sec-fetch-site"),
        Some(site)
            if site.eq_ignore_ascii_case("cross-site")
    ) {
        return false;
    }

    let Some(origin_raw) = headers.get(header::ORIGIN) else {
        // Non-browser clients may omit Origin (for example curl or internal tooling).
        // We only treat explicit browser cross-site signals as forbidden above.
        return true;
    };
    let Ok(origin) = origin_raw.to_str() else {
        return false;
    };
    let Ok(origin_url) = Url::parse(origin) else {
        return false;
    };
    if !matches!(origin_url.scheme(), "http" | "https") {
        return false;
    }

    let Some(origin_host) = origin_url.host_str() else {
        return false;
    };
    let Some((request_host, request_port)) =
        forwarded_or_host_authority(headers, origin_url.scheme())
    else {
        return false;
    };

    let origin_port = origin_url.port_or_known_default();
    if origin_host.eq_ignore_ascii_case(&request_host) && origin_port == request_port {
        return true;
    }

    // Dev loopback proxies (for example Vite on 60080 -> backend on 8080) may rewrite Host and/or port,
    // but both ends remain loopback. Allow that local-only mismatch.
    //
    // For non-loopback deployments behind reverse proxies, we accept trusted forwarded
    // host/proto/port headers for origin matching, but these headers are never relayed
    // to upstream/downstream proxy traffic (see should_proxy_header).
    is_loopback_authority_host(origin_host) && is_loopback_authority_host(&request_host)
}

pub(crate) fn forwarded_or_host_authority(
    headers: &HeaderMap,
    origin_scheme: &str,
) -> Option<(String, Option<u16>)> {
    if let Some(forwarded_host_raw) = header_value_as_str(headers, "x-forwarded-host") {
        // This service expects a single trusted edge gateway. If forwarded headers
        // arrive as a chain, treat it as unsupported/misconfigured and reject writes.
        let forwarded_host = single_forwarded_header_value(forwarded_host_raw)?;
        let authority = Authority::from_str(forwarded_host).ok()?;
        let forwarded_proto = match header_value_as_str(headers, "x-forwarded-proto") {
            Some(raw) => {
                let proto = single_forwarded_header_value(raw)?.to_ascii_lowercase();
                if proto == "http" || proto == "https" {
                    Some(proto)
                } else {
                    return None;
                }
            }
            None => None,
        };
        let scheme = forwarded_proto.as_deref().unwrap_or(origin_scheme);
        let forwarded_port = match header_value_as_str(headers, "x-forwarded-port") {
            Some(raw) => {
                let value = single_forwarded_header_value(raw)?;
                Some(value.parse::<u16>().ok()?)
            }
            None => None,
        };
        let port = authority
            .port_u16()
            .or(forwarded_port)
            .or_else(|| default_port_for_scheme(scheme));
        return Some((authority.host().to_string(), port));
    }

    if let Some(forwarded_raw) = header_value_as_str(headers, "forwarded")
        && let Some(forwarded_host) = forwarded_header_param(forwarded_raw, "host")
    {
        let authority = Authority::from_str(&forwarded_host).ok()?;
        let scheme =
            forwarded_header_param(forwarded_raw, "proto").map(|value| value.to_ascii_lowercase());
        let scheme = scheme.as_deref().unwrap_or(origin_scheme);
        let port = authority
            .port_u16()
            .or_else(|| default_port_for_scheme(scheme));
        return Some((authority.host().to_string(), port));
    }

    let host_raw = headers.get(header::HOST)?;
    let host_value = host_raw.to_str().ok()?;
    let authority = Authority::from_str(host_value).ok()?;
    Some((
        authority.host().to_string(),
        authority
            .port_u16()
            .or_else(|| default_port_for_scheme(origin_scheme)),
    ))
}

pub(crate) fn single_forwarded_header_value(raw: &str) -> Option<&str> {
    let mut parts = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let first = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(first)
}

pub(crate) fn forwarded_header_param(raw: &str, key: &str) -> Option<String> {
    let entry = single_forwarded_header_value(raw)?;
    for segment in entry.split(';') {
        let pair = segment.trim();
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name.eq_ignore_ascii_case(key) {
            let normalized = value.trim().trim_matches('"');
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
            return None;
        }
    }
    None
}

pub(crate) fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

pub(crate) fn header_value_as_str<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Option<&'a str> {
    headers
        .get(HeaderName::from_static(name))
        .and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod payload_utils_tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, header};

    #[test]
    fn generated_proxy_invoke_id_uses_short_nanoid_contract() {
        let invoke_id = generate_proxy_invoke_id();

        assert_eq!(invoke_id.len(), PROXY_INVOKE_ID_LENGTH);
        assert!(proxy_invoke_id_has_short_format(&invoke_id));
        assert!(!invoke_id.contains("proxy"));
        assert!(!invoke_id.contains('-'));
    }

    #[test]
    fn pool_routing_reservation_key_parses_only_legacy_proxy_invoke_ids() {
        assert_eq!(
            pool_routing_reservation_key_for_invoke_id("proxy-9061-1783013997090").as_deref(),
            Some("pool-route-9061")
        );
        assert_eq!(
            pool_routing_reservation_key_for_invoke_id("K7QM9ZD4HP"),
            None
        );
    }

    #[test]
    fn request_public_origin_defaults_to_http_host_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:8080"));

        assert_eq!(
            request_public_origin(&headers, None).as_deref(),
            Some("http://localhost:8080")
        );
    }

    #[test]
    fn request_public_origin_prefers_forwarded_https_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        headers.insert(
            header::HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("monitor.example.com"),
        );
        headers.insert(
            header::HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );

        assert_eq!(
            request_public_origin(&headers, None).as_deref(),
            Some("https://monitor.example.com")
        );
    }

    #[test]
    fn request_public_origin_defaults_to_http_when_forwarded_scheme_is_missing() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        headers.insert(
            header::HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("monitor.example.com"),
        );

        assert_eq!(
            request_public_origin(&headers, None).as_deref(),
            Some("http://monitor.example.com")
        );
    }

    #[test]
    fn request_public_origin_supports_standard_forwarded_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        headers.insert(
            header::HeaderName::from_static("forwarded"),
            HeaderValue::from_static("proto=https;host=monitor.example.com"),
        );

        assert_eq!(
            request_public_origin(&headers, None).as_deref(),
            Some("https://monitor.example.com")
        );
    }

    #[test]
    fn request_public_origin_defaults_to_http_for_public_host_without_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static("monitor.example.com"),
        );

        assert_eq!(
            request_public_origin(&headers, None).as_deref(),
            Some("http://monitor.example.com")
        );
    }

    #[test]
    fn request_public_origin_prefers_explicit_configured_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        headers.insert(
            header::HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("monitor.example.com"),
        );

        assert_eq!(
            request_public_origin(&headers, Some("https://monitor.example.com")).as_deref(),
            Some("https://monitor.example.com")
        );
    }
}
