pub(crate) fn default_forward_proxy_subscription_interval_secs() -> u64 {
    DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS
}

pub(crate) fn default_forward_proxy_insert_direct_compat() -> bool {
    true
}

pub(crate) fn decode_string_vec_json(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized).unwrap_or_default(),
        None => Vec::new(),
    }
}

pub(crate) fn normalize_subscription_entries(raw_entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for entry in raw_entries {
        for token in split_proxy_entry_tokens(&entry) {
            let Ok(url) = Url::parse(token) else {
                continue;
            };
            if !matches!(url.scheme(), "http" | "https") {
                continue;
            }
            let canonical = url.to_string();
            if seen.insert(canonical.clone()) {
                normalized.push(canonical);
            }
        }
    }
    normalized
}

pub(crate) fn normalize_proxy_url_entries(raw_entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for entry in raw_entries {
        for token in split_proxy_entry_tokens(&entry) {
            if let Some(parsed) = parse_forward_proxy_entry(token)
                && seen.insert(parsed.normalized.clone())
            {
                normalized.push(parsed.normalized);
            }
        }
    }
    normalized
}

pub(crate) fn split_proxy_entry_tokens(raw: &str) -> Vec<&str> {
    raw.split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.starts_with('#'))
        .collect()
}

#[cfg(test)]
pub(crate) fn normalize_single_proxy_url(raw: &str) -> Option<String> {
    parse_forward_proxy_entry(raw).map(|entry| entry.normalized)
}

pub(crate) fn normalize_single_proxy_key(raw: &str) -> Option<String> {
    parse_forward_proxy_entry(raw).map(|entry| entry.stable_key)
}

pub(crate) fn stable_forward_proxy_binding_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut stable = String::from("fpb_");
    for byte in digest.iter().take(16) {
        stable.push_str(&format!("{byte:02x}"));
    }
    stable
}

pub(crate) fn is_stable_forward_proxy_key(raw: &str) -> bool {
    raw.strip_prefix("fpn_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(crate) fn is_stable_forward_proxy_binding_key(raw: &str) -> bool {
    raw.strip_prefix("fpb_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(crate) fn normalize_bound_proxy_key(raw: &str) -> Option<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized == FORWARD_PROXY_DIRECT_KEY
        || is_stable_forward_proxy_key(normalized)
        || is_stable_forward_proxy_binding_key(normalized)
    {
        return Some(normalized.to_string());
    }
    normalize_single_proxy_key(normalized)
}

pub(crate) fn legacy_bound_proxy_key_aliases(
    raw: &str,
    protocol: ForwardProxyProtocol,
) -> Vec<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Vec::new();
    }

    let scheme = match protocol {
        ForwardProxyProtocol::Vless => Some("vless"),
        ForwardProxyProtocol::Trojan => Some("trojan"),
        _ => None,
    };
    let Some(scheme) = scheme else {
        return Vec::new();
    };

    let Some(parsed) = Url::parse(normalized).ok() else {
        return Vec::new();
    };
    if !parsed.scheme().eq_ignore_ascii_case(scheme) {
        return Vec::new();
    }

    let default_specs = match protocol {
        ForwardProxyProtocol::Vless => &[
            LegacyDefaultQueryParamSpec {
                keys: &["encryption"],
                explicit_keys: &["encryption"],
                default_value: Some("none"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["security"],
                explicit_keys: &["security"],
                default_value: Some("none"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["type", "net"],
                explicit_keys: &["type", "net"],
                default_value: Some("tcp"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["sni", "serverName"],
                explicit_keys: &["sni", "serverName"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["fp", "fingerprint"],
                explicit_keys: &["fp", "fingerprint"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["serviceName", "service_name"],
                explicit_keys: &["serviceName", "service_name"],
                default_value: None,
            },
        ][..],
        ForwardProxyProtocol::Trojan => &[
            LegacyDefaultQueryParamSpec {
                keys: &["security"],
                explicit_keys: &["security"],
                default_value: Some("tls"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["type", "net"],
                explicit_keys: &["type", "net"],
                default_value: Some("tcp"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["sni", "serverName"],
                explicit_keys: &["sni", "serverName"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["fp", "fingerprint"],
                explicit_keys: &["fp", "fingerprint"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["serviceName", "service_name"],
                explicit_keys: &["serviceName", "service_name"],
                default_value: None,
            },
        ][..],
        _ => &[][..],
    };

    let mut aliases = legacy_share_link_identity_variants(&parsed, default_specs)
        .into_iter()
        .map(|identity| stable_forward_proxy_key(&identity))
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

pub(crate) fn forward_proxy_storage_aliases(raw: &str) -> Option<(String, Vec<String>)> {
    let parsed = parse_forward_proxy_entry(raw)?;
    let canonical = parsed.stable_key.clone();
    let mut aliases = Vec::new();
    if parsed.normalized != canonical {
        aliases.push(parsed.normalized.clone());
    }
    if matches!(
        parsed.protocol,
        ForwardProxyProtocol::Vless | ForwardProxyProtocol::Trojan
    ) {
        aliases.extend(legacy_bound_proxy_key_aliases(
            &parsed.normalized,
            parsed.protocol,
        ));
    }
    aliases.retain(|alias| alias != &canonical);
    aliases.sort();
    aliases.dedup();
    Some((canonical, aliases))
}

pub(crate) fn normalize_proxy_endpoints_from_urls(
    urls: &[String],
    source: &str,
) -> Vec<ForwardProxyEndpoint> {
    let mut seen = HashSet::new();
    let mut endpoints = Vec::new();
    for raw in urls {
        if let Some(parsed) = parse_forward_proxy_entry(raw) {
            let key = parsed.stable_key.clone();
            if !seen.insert(key.clone()) {
                continue;
            }
            endpoints.push(ForwardProxyEndpoint {
                key,
                source: source.to_string(),
                display_name: parsed.display_name,
                protocol: parsed.protocol,
                endpoint_url: parsed.endpoint_url,
                raw_url: Some(parsed.normalized),
            });
        }
    }
    endpoints
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedForwardProxyEntry {
    pub(crate) normalized: String,
    pub(crate) stable_key: String,
    pub(crate) display_name: String,
    pub(crate) protocol: ForwardProxyProtocol,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) endpoint_url: Option<Url>,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyBindingParts {
    pub(crate) display_name: String,
    pub(crate) protocol_key: String,
    pub(crate) host_port: String,
}

pub(crate) fn parse_forward_proxy_entry(raw: &str) -> Option<ParsedForwardProxyEntry> {
    let candidate = raw.trim();
    if candidate.is_empty() {
        return None;
    }

    if !candidate.contains("://") {
        return parse_native_forward_proxy(&format!("http://{candidate}"));
    }

    let (scheme_raw, _) = candidate.split_once("://")?;
    let scheme = scheme_raw.to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" | "socks5" | "socks5h" | "socks" => parse_native_forward_proxy(candidate),
        "vmess" => parse_vmess_forward_proxy(candidate),
        "vless" => parse_vless_forward_proxy(candidate),
        "trojan" => parse_trojan_forward_proxy(candidate),
        "ss" => parse_shadowsocks_forward_proxy(candidate),
        _ => None,
    }
}

pub(crate) fn parse_native_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let parsed = Url::parse(candidate).ok()?;
    let raw_scheme = parsed.scheme();
    let (protocol, normalized_scheme) = match raw_scheme {
        "http" => (ForwardProxyProtocol::Http, "http"),
        "https" => (ForwardProxyProtocol::Https, "https"),
        "socks5" | "socks" => (ForwardProxyProtocol::Socks5, "socks5"),
        "socks5h" => (ForwardProxyProtocol::Socks5h, "socks5h"),
        _ => return None,
    };

    let host = parsed.host_str()?;
    if host.trim().is_empty() {
        return None;
    }
    let port = parsed.port_or_known_default()?;
    let mut normalized = format!("{normalized_scheme}://");
    if !parsed.username().is_empty() {
        normalized.push_str(parsed.username());
        if let Some(password) = parsed.password() {
            normalized.push(':');
            normalized.push_str(password);
        }
        normalized.push('@');
    }
    if host.contains(':') {
        normalized.push('[');
        normalized.push_str(host);
        normalized.push(']');
    } else {
        normalized.push_str(&host.to_ascii_lowercase());
    }
    normalized.push(':');
    normalized.push_str(&port.to_string());
    let endpoint_url = Url::parse(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        stable_key: stable_forward_proxy_key(&normalized),
        normalized,
        display_name: format!("{host}:{port}"),
        protocol,
        host: host.to_ascii_lowercase(),
        port,
        endpoint_url: Some(endpoint_url),
    })
}

pub(crate) fn parse_vmess_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "vmess")?;
    let parsed = parse_vmess_share_link(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        stable_key: stable_forward_proxy_key(&parsed.stable_identity()),
        normalized,
        display_name: parsed.display_name,
        protocol: ForwardProxyProtocol::Vmess,
        host: parsed.address.to_ascii_lowercase(),
        port: parsed.port,
        endpoint_url: None,
    })
}

pub(crate) fn parse_vless_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "vless")?;
    let parsed = Url::parse(&normalized).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let display_name =
        proxy_display_name_from_url(&parsed).unwrap_or_else(|| format!("{host}:{port}"));
    Some(ParsedForwardProxyEntry {
        stable_key: stable_forward_proxy_key(&canonical_vless_share_link_identity(&parsed)),
        normalized,
        display_name,
        protocol: ForwardProxyProtocol::Vless,
        host: host.to_ascii_lowercase(),
        port,
        endpoint_url: None,
    })
}

pub(crate) fn parse_trojan_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "trojan")?;
    let parsed = Url::parse(&normalized).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let display_name =
        proxy_display_name_from_url(&parsed).unwrap_or_else(|| format!("{host}:{port}"));
    Some(ParsedForwardProxyEntry {
        stable_key: stable_forward_proxy_key(&canonical_trojan_share_link_identity(&parsed)),
        normalized,
        display_name,
        protocol: ForwardProxyProtocol::Trojan,
        host: host.to_ascii_lowercase(),
        port,
        endpoint_url: None,
    })
}

pub(crate) fn parse_shadowsocks_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "ss")?;
    let parsed = parse_shadowsocks_share_link(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        stable_key: Url::parse(&normalized)
            .ok()
            .map(|url| stable_forward_proxy_key(&canonical_share_link_identity(&url)))
            .unwrap_or_else(|| stable_forward_proxy_key(&parsed.stable_identity())),
        normalized,
        display_name: parsed.display_name,
        protocol: ForwardProxyProtocol::Shadowsocks,
        host: parsed.host.to_ascii_lowercase(),
        port: parsed.port,
        endpoint_url: None,
    })
}

pub(crate) fn canonical_host_port_string(host: &str, port: u16) -> String {
    let normalized_host = host.trim().to_ascii_lowercase();
    if normalized_host.contains(':') {
        format!("[{normalized_host}]:{port}")
    } else {
        format!("{normalized_host}:{port}")
    }
}

pub(crate) fn forward_proxy_binding_parts_from_raw(
    raw: &str,
    display_name_override: Option<&str>,
) -> Option<ForwardProxyBindingParts> {
    let parsed = parse_forward_proxy_entry(raw)?;
    let display_name = display_name_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(parsed.display_name.as_str())
        .trim()
        .to_string();
    if display_name.is_empty() {
        return None;
    }
    Some(ForwardProxyBindingParts {
        display_name,
        protocol_key: parsed.protocol.label().to_string(),
        host_port: canonical_host_port_string(&parsed.host, parsed.port),
    })
}

pub(crate) fn forward_proxy_binding_key_candidates(
    parts: &ForwardProxyBindingParts,
) -> [String; 3] {
    [
        stable_forward_proxy_binding_key(&format!("name:{}", parts.display_name)),
        stable_forward_proxy_binding_key(&format!(
            "name:{}|protocol:{}",
            parts.display_name, parts.protocol_key
        )),
        stable_forward_proxy_binding_key(&format!(
            "name:{}|protocol:{}|server:{}",
            parts.display_name, parts.protocol_key, parts.host_port
        )),
    ]
}

pub(crate) fn proxy_display_name_from_url(url: &Url) -> Option<String> {
    if let Some(fragment) = url.fragment()
        && !fragment.trim().is_empty()
    {
        return Some(percent_decode_once_lossy(fragment));
    }
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

pub(crate) fn normalize_share_link_scheme(candidate: &str, scheme: &str) -> Option<String> {
    let (_, remainder) = candidate.split_once("://")?;
    let normalized = format!("{scheme}://{}", remainder.trim());
    if normalized.len() <= scheme.len() + 3 {
        return None;
    }
    Some(normalized)
}

pub(crate) fn stable_forward_proxy_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut stable = String::from("fpn_");
    for byte in digest.iter().take(16) {
        stable.push_str(&format!("{byte:02x}"));
    }
    stable
}

pub(crate) fn push_canonical_host_port(identity: &mut String, host: &str, port: u16) {
    if host.contains(':') {
        identity.push('[');
        identity.push_str(host);
        identity.push(']');
    } else {
        identity.push_str(host);
    }
    identity.push(':');
    identity.push_str(&port.to_string());
}

pub(crate) fn normalized_query_value(
    query: &HashMap<String, String>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| query.get(*key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn normalized_query_ascii_lowercase(
    query: &HashMap<String, String>,
    keys: &[&str],
) -> Option<String> {
    normalized_query_value(query, keys).map(|value| value.to_ascii_lowercase())
}

pub(crate) fn sorted_query_pairs(url: &Url) -> Vec<(String, String)> {
    let mut query_pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    query_pairs.sort();
    query_pairs
}

pub(crate) fn canonical_query_string(query_pairs: Vec<(String, String)>) -> String {
    query_pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

#[derive(Clone, Copy)]
pub(crate) struct LegacyDefaultQueryParamSpec {
    pub(crate) keys: &'static [&'static str],
    pub(crate) explicit_keys: &'static [&'static str],
    pub(crate) default_value: Option<&'static str>,
}

pub(crate) fn build_legacy_query_param_variant_choices(
    matching_pairs: &[(String, String)],
    spec: &LegacyDefaultQueryParamSpec,
) -> Option<Vec<Vec<(String, String)>>> {
    let shared_value = if let Some((_, value)) = matching_pairs.first() {
        if !matching_pairs
            .iter()
            .all(|(_, candidate)| candidate.trim().eq_ignore_ascii_case(value.trim()))
        {
            return None;
        }
        value.trim().to_string()
    } else {
        spec.default_value?.to_string()
    };

    let explicit_keys = if spec.explicit_keys.is_empty() {
        spec.keys
    } else {
        spec.explicit_keys
    };
    let mut choices = Vec::new();
    if spec
        .default_value
        .is_some_and(|default_value| shared_value.eq_ignore_ascii_case(default_value))
    {
        choices.push(Vec::new());
    }
    for mask in 1usize..(1usize << explicit_keys.len()) {
        let mut pairs = Vec::new();
        for (index, key) in explicit_keys.iter().enumerate() {
            if (mask & (1usize << index)) != 0 {
                pairs.push(((*key).to_string(), shared_value.clone()));
            }
        }
        choices.push(pairs);
    }
    Some(choices)
}

pub(crate) fn legacy_share_link_identity_variants(
    url: &Url,
    default_specs: &[LegacyDefaultQueryParamSpec],
) -> Vec<String> {
    let original_query_pairs = sorted_query_pairs(url);
    let mut static_pairs = Vec::new();
    let mut handled_keys = HashSet::new();
    let mut variant_choices: Vec<Vec<Vec<(String, String)>>> = Vec::new();

    for spec in default_specs {
        let matching_pairs = original_query_pairs
            .iter()
            .filter(|(key, _)| spec.keys.contains(&key.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        for key in spec.keys {
            handled_keys.insert(*key);
        }

        let Some(choices) = build_legacy_query_param_variant_choices(&matching_pairs, spec) else {
            static_pairs.extend(matching_pairs);
            continue;
        };
        variant_choices.push(choices);
    }

    static_pairs.extend(
        original_query_pairs
            .into_iter()
            .filter(|(key, _)| !handled_keys.contains(key.as_str())),
    );
    static_pairs.sort();

    let mut variants = vec![static_pairs];
    for choices in variant_choices {
        let mut next = Vec::new();
        let mut seen = HashSet::new();
        for variant in &variants {
            for choice in &choices {
                let mut updated = variant.clone();
                updated.extend(choice.iter().cloned());
                updated.sort();
                let query = canonical_query_string(updated.clone());
                if seen.insert(query) {
                    next.push(updated);
                }
            }
        }
        variants = next;
    }

    variants
        .into_iter()
        .map(|query_pairs| share_link_identity_with_query_pairs(url, query_pairs))
        .collect()
}

struct CanonicalStreamProperties {
    network: String,
    security: String,
    host: String,
    path: String,
    service_name: String,
}

fn canonical_stream_properties(
    query: &HashMap<String, String>,
    default_security: Option<&str>,
) -> CanonicalStreamProperties {
    let network = normalized_query_ascii_lowercase(query, &["type", "net"])
        .unwrap_or_else(|| "tcp".to_string());
    let security = normalized_query_ascii_lowercase(query, &["security"])
        .or_else(|| default_security.map(str::to_ascii_lowercase))
        .unwrap_or_else(|| "none".to_string());
    let host = normalized_query_value(query, &["host"])
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let path = normalized_query_value(query, &["path"]).unwrap_or_default();
    let service_name = normalized_query_value(query, &["serviceName", "service_name"])
        .or_else(|| (!path.is_empty()).then_some(path.clone()))
        .unwrap_or_default();
    CanonicalStreamProperties {
        network,
        security,
        host,
        path,
        service_name,
    }
}

fn append_canonical_network_query_pairs(
    query: &HashMap<String, String>,
    properties: &CanonicalStreamProperties,
    consumed: &mut HashSet<&'static str>,
    pairs: &mut Vec<(String, String)>,
) {
    match properties.network.as_str() {
        "ws" | "httpupgrade" => {
            consumed.extend(["host", "path"]);
            pairs.extend([
                ("host".to_string(), properties.host.clone()),
                ("path".to_string(), properties.path.clone()),
            ]);
        }
        "grpc" => {
            consumed.extend(["serviceName", "service_name", "multiMode"]);
            pairs.push(("serviceName".to_string(), properties.service_name.clone()));
            pairs.push((
                "multiMode".to_string(),
                query_flag_true(query, "multiMode").to_string(),
            ));
        }
        _ => {}
    }
}

fn canonical_security_server_name(
    url: &Url,
    query: &HashMap<String, String>,
    host: &str,
) -> String {
    normalized_query_value(query, &["sni", "serverName"])
        .map(|value| value.to_ascii_lowercase())
        .or_else(|| (!host.is_empty()).then_some(host.to_string()))
        .or_else(|| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default()
}

fn append_canonical_security_query_pairs(
    url: &Url,
    query: &HashMap<String, String>,
    properties: &CanonicalStreamProperties,
    consumed: &mut HashSet<&'static str>,
    pairs: &mut Vec<(String, String)>,
) {
    match properties.security.as_str() {
        "tls" => {
            consumed.extend([
                "sni",
                "serverName",
                "allowInsecure",
                "insecure",
                "fp",
                "fingerprint",
                "alpn",
            ]);
            let alpn = normalized_query_value(query, &["alpn"])
                .map(|value| {
                    parse_alpn_csv(&value)
                        .into_iter()
                        .map(|item| item.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            pairs.extend([
                ("alpn".to_string(), alpn),
                (
                    "allowInsecure".to_string(),
                    (query_flag_true(query, "allowInsecure") || query_flag_true(query, "insecure"))
                        .to_string(),
                ),
                (
                    "fp".to_string(),
                    normalized_query_ascii_lowercase(query, &["fp", "fingerprint"])
                        .unwrap_or_default(),
                ),
                (
                    "serverName".to_string(),
                    canonical_security_server_name(url, query, &properties.host),
                ),
            ]);
        }
        "reality" => {
            consumed.extend([
                "sni",
                "serverName",
                "fp",
                "fingerprint",
                "pbk",
                "sid",
                "spx",
            ]);
            pairs.extend([
                (
                    "fp".to_string(),
                    normalized_query_ascii_lowercase(query, &["fp", "fingerprint"])
                        .unwrap_or_default(),
                ),
                (
                    "pbk".to_string(),
                    normalized_query_value(query, &["pbk"]).unwrap_or_default(),
                ),
                (
                    "serverName".to_string(),
                    canonical_security_server_name(url, query, &properties.host),
                ),
                (
                    "sid".to_string(),
                    normalized_query_value(query, &["sid"]).unwrap_or_default(),
                ),
                (
                    "spx".to_string(),
                    normalized_query_value(query, &["spx"]).unwrap_or_default(),
                ),
            ]);
        }
        _ => {}
    }
}

pub(crate) fn canonical_stream_query_pairs(
    url: &Url,
    default_security: Option<&str>,
    consumed_keys: &mut HashSet<&'static str>,
) -> Vec<(String, String)> {
    let original_query_pairs = sorted_query_pairs(url);
    let query = original_query_pairs
        .iter()
        .cloned()
        .collect::<HashMap<String, String>>();
    let properties = canonical_stream_properties(&query, default_security);
    consumed_keys.extend(["type", "net", "security"]);
    let mut pairs = vec![
        ("net".to_string(), properties.network.clone()),
        ("security".to_string(), properties.security.clone()),
    ];
    append_canonical_network_query_pairs(&query, &properties, consumed_keys, &mut pairs);
    append_canonical_security_query_pairs(url, &query, &properties, consumed_keys, &mut pairs);
    pairs.extend(
        original_query_pairs
            .into_iter()
            .filter(|(key, _)| !consumed_keys.contains(key.as_str())),
    );
    pairs.sort();
    pairs
}

pub(crate) fn canonical_stream_identity_from_url(
    url: &Url,
    default_security: Option<&str>,
    consumed_keys: &mut HashSet<&'static str>,
) -> String {
    canonical_query_string(canonical_stream_query_pairs(
        url,
        default_security,
        consumed_keys,
    ))
}

pub(crate) fn canonical_vless_share_link_identity(url: &Url) -> String {
    let user_id = percent_decode_once_lossy(url.username());
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or_default();
    let query_pairs = sorted_query_pairs(url);
    let query = query_pairs
        .iter()
        .cloned()
        .collect::<HashMap<String, String>>();
    let encryption = normalized_query_ascii_lowercase(&query, &["encryption"])
        .unwrap_or_else(|| "none".to_string());
    let flow = normalized_query_value(&query, &["flow"]).unwrap_or_default();

    let mut consumed_keys = HashSet::from(["encryption", "flow"]);
    let mut canonical_query_pairs = vec![
        ("encryption".to_string(), encryption),
        ("flow".to_string(), flow),
    ];
    canonical_query_pairs.extend(canonical_stream_query_pairs(url, None, &mut consumed_keys));
    canonical_query_pairs.sort();

    let mut identity = String::from("vless://");
    identity.push_str(&user_id);
    identity.push('@');
    push_canonical_host_port(&mut identity, &host, port);
    identity.push('?');
    identity.push_str(&canonical_query_string(canonical_query_pairs));
    identity
}

pub(crate) fn canonical_trojan_share_link_identity(url: &Url) -> String {
    let password = percent_decode_once_lossy(url.username());
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or_default();

    let mut identity = String::from("trojan://");
    identity.push_str(&password);
    identity.push('@');
    push_canonical_host_port(&mut identity, &host, port);
    identity.push('?');
    identity.push_str(&canonical_stream_identity_from_url(
        url,
        Some("tls"),
        &mut HashSet::new(),
    ));
    identity
}

pub(crate) fn canonical_share_link_identity(url: &Url) -> String {
    share_link_identity_with_query_pairs(url, sorted_query_pairs(url))
}

pub(crate) fn share_link_identity_with_query_pairs(
    url: &Url,
    query_pairs: Vec<(String, String)>,
) -> String {
    let scheme = url.scheme().to_ascii_lowercase();
    let username = percent_decode_once_lossy(url.username());
    let password = url.password().map(percent_decode_once_lossy);
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or_default();
    let path = url.path();
    let query = canonical_query_string(query_pairs);

    let mut identity = format!("{scheme}://");
    if !username.is_empty() {
        identity.push_str(&username);
        if let Some(password) = password {
            identity.push(':');
            identity.push_str(&password);
        }
        identity.push('@');
    }
    if host.contains(':') {
        identity.push('[');
        identity.push_str(&host);
        identity.push(']');
    } else {
        identity.push_str(&host);
    }
    identity.push(':');
    identity.push_str(&port.to_string());
    if !path.is_empty() && path != "/" {
        identity.push_str(path);
    }
    if !query.is_empty() {
        identity.push('?');
        identity.push_str(&query);
    }
    identity
}

pub(crate) fn decode_base64_any(raw: &str) -> Option<Vec<u8>> {
    let compact = raw
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if compact.is_empty() {
        return None;
    }
    for engine in [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(compact.as_bytes()) {
            return Some(decoded);
        }
    }
    None
}

pub(crate) fn decode_base64_string(raw: &str) -> Option<String> {
    decode_base64_any(raw).and_then(|bytes| String::from_utf8(bytes).ok())
}
