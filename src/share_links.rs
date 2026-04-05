fn default_forward_proxy_subscription_interval_secs() -> u64 {
    DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS
}

fn default_forward_proxy_insert_direct_compat() -> bool {
    true
}

fn decode_string_vec_json(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized).unwrap_or_default(),
        None => Vec::new(),
    }
}

fn normalize_subscription_entries(raw_entries: Vec<String>) -> Vec<String> {
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

fn normalize_proxy_url_entries(raw_entries: Vec<String>) -> Vec<String> {
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

fn split_proxy_entry_tokens(raw: &str) -> Vec<&str> {
    raw.split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.starts_with('#'))
        .collect()
}

#[cfg(test)]
fn normalize_single_proxy_url(raw: &str) -> Option<String> {
    parse_forward_proxy_entry(raw).map(|entry| entry.normalized)
}

fn normalize_single_proxy_key(raw: &str) -> Option<String> {
    parse_forward_proxy_entry(raw).map(|entry| entry.stable_key)
}

fn stable_forward_proxy_binding_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut stable = String::from("fpb_");
    for byte in digest.iter().take(16) {
        stable.push_str(&format!("{byte:02x}"));
    }
    stable
}

fn is_stable_forward_proxy_key(raw: &str) -> bool {
    raw.strip_prefix("fpn_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn is_stable_forward_proxy_binding_key(raw: &str) -> bool {
    raw.strip_prefix("fpb_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn normalize_bound_proxy_key(raw: &str) -> Option<String> {
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

fn normalize_proxy_endpoints_from_urls(urls: &[String], source: &str) -> Vec<ForwardProxyEndpoint> {
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
struct ParsedForwardProxyEntry {
    normalized: String,
    stable_key: String,
    display_name: String,
    protocol: ForwardProxyProtocol,
    host: String,
    port: u16,
    endpoint_url: Option<Url>,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyBindingParts {
    pub(crate) display_name: String,
    pub(crate) protocol_key: String,
    pub(crate) host_port: String,
}

fn parse_forward_proxy_entry(raw: &str) -> Option<ParsedForwardProxyEntry> {
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

fn parse_native_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
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

fn parse_vmess_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
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

fn parse_vless_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
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

fn parse_trojan_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
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

fn parse_shadowsocks_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
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

fn canonical_host_port_string(host: &str, port: u16) -> String {
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

fn proxy_display_name_from_url(url: &Url) -> Option<String> {
    if let Some(fragment) = url.fragment()
        && !fragment.trim().is_empty()
    {
        return Some(percent_decode_once_lossy(fragment));
    }
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

fn normalize_share_link_scheme(candidate: &str, scheme: &str) -> Option<String> {
    let (_, remainder) = candidate.split_once("://")?;
    let normalized = format!("{scheme}://{}", remainder.trim());
    if normalized.len() <= scheme.len() + 3 {
        return None;
    }
    Some(normalized)
}

fn stable_forward_proxy_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut stable = String::from("fpn_");
    for byte in digest.iter().take(16) {
        stable.push_str(&format!("{byte:02x}"));
    }
    stable
}

fn push_canonical_host_port(identity: &mut String, host: &str, port: u16) {
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

fn normalized_query_value(query: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| query.get(*key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalized_query_ascii_lowercase(
    query: &HashMap<String, String>,
    keys: &[&str],
) -> Option<String> {
    normalized_query_value(query, keys).map(|value| value.to_ascii_lowercase())
}

fn sorted_query_pairs(url: &Url) -> Vec<(String, String)> {
    let mut query_pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    query_pairs.sort();
    query_pairs
}

fn canonical_query_string(query_pairs: Vec<(String, String)>) -> String {
    query_pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

#[derive(Clone, Copy)]
struct LegacyDefaultQueryParamSpec {
    keys: &'static [&'static str],
    explicit_keys: &'static [&'static str],
    default_value: Option<&'static str>,
}

fn build_legacy_query_param_variant_choices(
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

fn legacy_share_link_identity_variants(
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

fn canonical_stream_query_pairs(
    url: &Url,
    default_security: Option<&str>,
    consumed_keys: &mut HashSet<&'static str>,
) -> Vec<(String, String)> {
    let original_query_pairs = sorted_query_pairs(url);
    let query = original_query_pairs
        .iter()
        .cloned()
        .collect::<HashMap<String, String>>();
    let network = normalized_query_ascii_lowercase(&query, &["type", "net"])
        .unwrap_or_else(|| "tcp".to_string());
    let security = normalized_query_ascii_lowercase(&query, &["security"])
        .or_else(|| default_security.map(|value| value.to_ascii_lowercase()))
        .unwrap_or_else(|| "none".to_string());
    let host = normalized_query_value(&query, &["host"])
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let path = normalized_query_value(&query, &["path"]).unwrap_or_default();
    let service_name = normalized_query_value(&query, &["serviceName", "service_name"])
        .or_else(|| (!path.is_empty()).then_some(path.clone()))
        .unwrap_or_default();

    consumed_keys.extend(["type", "net", "security"]);
    let mut query_pairs = vec![
        ("net".to_string(), network.clone()),
        ("security".to_string(), security.clone()),
    ];

    match network.as_str() {
        "ws" => {
            consumed_keys.extend(["host", "path"]);
            query_pairs.push(("host".to_string(), host.clone()));
            query_pairs.push(("path".to_string(), path.clone()));
        }
        "grpc" => {
            consumed_keys.extend(["serviceName", "service_name", "multiMode"]);
            query_pairs.push(("serviceName".to_string(), service_name.clone()));
            query_pairs.push((
                "multiMode".to_string(),
                if query_flag_true(&query, "multiMode") {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ));
        }
        "httpupgrade" => {
            consumed_keys.extend(["host", "path"]);
            query_pairs.push(("host".to_string(), host.clone()));
            query_pairs.push(("path".to_string(), path.clone()));
        }
        _ => {}
    }

    match security.as_str() {
        "tls" => {
            consumed_keys.extend([
                "sni",
                "serverName",
                "allowInsecure",
                "insecure",
                "fp",
                "fingerprint",
                "alpn",
            ]);
            let server_name = normalized_query_value(&query, &["sni", "serverName"])
                .map(|value| value.to_ascii_lowercase())
                .or_else(|| (!host.is_empty()).then_some(host.clone()))
                .or_else(|| url.host_str().map(|value| value.to_ascii_lowercase()))
                .unwrap_or_default();
            let fingerprint = normalized_query_ascii_lowercase(&query, &["fp", "fingerprint"])
                .unwrap_or_default();
            let alpn = normalized_query_value(&query, &["alpn"])
                .map(|value| {
                    parse_alpn_csv(&value)
                        .into_iter()
                        .map(|item| item.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            query_pairs.push(("alpn".to_string(), alpn));
            query_pairs.push((
                "allowInsecure".to_string(),
                if query_flag_true(&query, "allowInsecure") || query_flag_true(&query, "insecure") {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ));
            query_pairs.push(("fp".to_string(), fingerprint));
            query_pairs.push(("serverName".to_string(), server_name));
        }
        "reality" => {
            consumed_keys.extend([
                "sni",
                "serverName",
                "fp",
                "fingerprint",
                "pbk",
                "sid",
                "spx",
            ]);
            let server_name = normalized_query_value(&query, &["sni", "serverName"])
                .map(|value| value.to_ascii_lowercase())
                .or_else(|| (!host.is_empty()).then_some(host.clone()))
                .or_else(|| url.host_str().map(|value| value.to_ascii_lowercase()))
                .unwrap_or_default();
            let fingerprint = normalized_query_ascii_lowercase(&query, &["fp", "fingerprint"])
                .unwrap_or_default();
            let public_key = normalized_query_value(&query, &["pbk"]).unwrap_or_default();
            let short_id = normalized_query_value(&query, &["sid"]).unwrap_or_default();
            let spider_x = normalized_query_value(&query, &["spx"]).unwrap_or_default();
            query_pairs.push(("fp".to_string(), fingerprint));
            query_pairs.push(("pbk".to_string(), public_key));
            query_pairs.push(("serverName".to_string(), server_name));
            query_pairs.push(("sid".to_string(), short_id));
            query_pairs.push(("spx".to_string(), spider_x));
        }
        _ => {}
    }

    query_pairs.extend(
        original_query_pairs
            .into_iter()
            .filter(|(key, _)| !consumed_keys.contains(key.as_str())),
    );
    query_pairs.sort();
    query_pairs
}

fn canonical_stream_identity_from_url(
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

fn canonical_vless_share_link_identity(url: &Url) -> String {
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

fn canonical_trojan_share_link_identity(url: &Url) -> String {
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

fn canonical_share_link_identity(url: &Url) -> String {
    share_link_identity_with_query_pairs(url, sorted_query_pairs(url))
}

fn share_link_identity_with_query_pairs(url: &Url, query_pairs: Vec<(String, String)>) -> String {
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

fn decode_base64_any(raw: &str) -> Option<Vec<u8>> {
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

fn decode_base64_string(raw: &str) -> Option<String> {
    decode_base64_any(raw).and_then(|bytes| String::from_utf8(bytes).ok())
}

#[derive(Debug, Clone)]
struct VmessShareLink {
    address: String,
    port: u16,
    id: String,
    alter_id: u32,
    security: String,
    network: String,
    host: Option<String>,
    path: Option<String>,
    tls_mode: Option<String>,
    sni: Option<String>,
    alpn: Option<Vec<String>>,
    fingerprint: Option<String>,
    header_type: Option<String>,
    service_name: Option<String>,
    authority: Option<String>,
    mode: Option<String>,
    seed: Option<String>,
    display_name: String,
}

impl VmessShareLink {
    fn stable_identity(&self) -> String {
        let alpn = self
            .alpn
            .as_ref()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        format!(
            "vmess://{}@{}:{}?aid={}&security={}&net={}&host={}&path={}&tls={}&sni={}&alpn={}&fp={}&type={}&serviceName={}&authority={}&mode={}&seed={}",
            self.id,
            self.address.to_ascii_lowercase(),
            self.port,
            self.alter_id,
            self.security.to_ascii_lowercase(),
            self.network.to_ascii_lowercase(),
            self.host
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.path.as_deref().unwrap_or_default(),
            self.tls_mode
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.sni.as_deref().unwrap_or_default().to_ascii_lowercase(),
            alpn,
            self.fingerprint
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.header_type
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.service_name.as_deref().unwrap_or_default(),
            self.authority.as_deref().unwrap_or_default(),
            self.mode
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.seed.as_deref().unwrap_or_default(),
        )
    }
}

fn parse_vmess_share_link(raw: &str) -> Result<VmessShareLink> {
    let payload = raw
        .strip_prefix("vmess://")
        .ok_or_else(|| anyhow!("invalid vmess share link"))?;
    let decoded =
        decode_base64_string(payload).ok_or_else(|| anyhow!("failed to decode vmess payload"))?;
    let value: Value = serde_json::from_str(&decoded).context("invalid vmess json payload")?;

    let address = value
        .get("add")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("vmess payload missing add"))?
        .to_string();
    let port =
        parse_port_value(value.get("port")).ok_or_else(|| anyhow!("vmess payload missing port"))?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("vmess payload missing id"))?
        .to_string();
    let alter_id = parse_u32_value(value.get("aid")).unwrap_or(0);
    let security = value
        .get("scy")
        .and_then(Value::as_str)
        .or_else(|| value.get("security").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("auto")
        .to_string();
    let network = value
        .get("net")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("tcp")
        .to_ascii_lowercase();
    let host = value
        .get("host")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let tls_mode = value
        .get("tls")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let sni = value
        .get("sni")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let alpn = value
        .get("alpn")
        .and_then(Value::as_str)
        .map(parse_alpn_csv)
        .filter(|items| !items.is_empty());
    let fingerprint = value
        .get("fp")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let header_type = value
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let service_name = value
        .get("serviceName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let authority = value
        .get("authority")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let mode = value
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let seed = value
        .get("seed")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let display_name = value
        .get("ps")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{address}:{port}"));

    Ok(VmessShareLink {
        address,
        port,
        id,
        alter_id,
        security,
        network,
        host,
        path,
        tls_mode,
        sni,
        alpn,
        fingerprint,
        header_type,
        service_name,
        authority,
        mode,
        seed,
        display_name,
    })
}

fn parse_u32_value(value: Option<&Value>) -> Option<u32> {
    match value {
        Some(Value::Number(num)) => num.as_u64().and_then(|v| u32::try_from(v).ok()),
        Some(Value::String(raw)) => raw.trim().parse::<u32>().ok(),
        _ => None,
    }
}

fn parse_port_value(value: Option<&Value>) -> Option<u16> {
    match value {
        Some(Value::Number(num)) => num.as_u64().and_then(|v| u16::try_from(v).ok()),
        Some(Value::String(raw)) => raw.trim().parse::<u16>().ok(),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ShadowsocksShareLink {
    method: String,
    password: String,
    host: String,
    port: u16,
    display_name: String,
}

impl ShadowsocksShareLink {
    fn stable_identity(&self) -> String {
        format!(
            "ss://{}:{}@{}:{}",
            self.method.to_ascii_lowercase(),
            self.password,
            self.host.to_ascii_lowercase(),
            self.port,
        )
    }
}

fn parse_shadowsocks_share_link(raw: &str) -> Result<ShadowsocksShareLink> {
    let normalized = raw
        .strip_prefix("ss://")
        .ok_or_else(|| anyhow!("invalid shadowsocks share link"))?;
    let (main, fragment) = split_once_first(normalized, '#');
    let (main, _) = split_once_first(main, '?');
    let display_name = fragment
        .map(percent_decode_once_lossy)
        .filter(|value| !value.trim().is_empty());

    if let Ok(url) = Url::parse(raw)
        && let Some(host) = url.host_str()
        && let Some(port) = url.port_or_known_default()
    {
        let credentials = if !url.username().is_empty() && url.password().is_some() {
            Some((
                percent_decode_once_lossy(url.username()),
                percent_decode_once_lossy(url.password().unwrap_or_default()),
            ))
        } else if !url.username().is_empty() {
            let username = percent_decode_once_lossy(url.username());
            decode_base64_string(&username).and_then(|decoded| {
                let (method, password) = decoded.split_once(':')?;
                Some((method.to_string(), password.to_string()))
            })
        } else {
            None
        };
        if let Some((method, password)) = credentials {
            return Ok(ShadowsocksShareLink {
                method,
                password,
                host: host.to_string(),
                port,
                display_name: display_name
                    .clone()
                    .unwrap_or_else(|| format!("{host}:{port}")),
            });
        }
    }

    let decoded_main = if main.contains('@') {
        main.to_string()
    } else {
        let main_for_decode = percent_decode_once_lossy(main);
        decode_base64_string(&main_for_decode)
            .ok_or_else(|| anyhow!("failed to decode shadowsocks payload"))?
    };

    let (credential, host_port) = decoded_main
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("invalid shadowsocks payload"))?;
    let (method, password) = if let Some((method, password)) = credential.split_once(':') {
        (
            percent_decode_once_lossy(method),
            percent_decode_once_lossy(password),
        )
    } else {
        let decoded_credential = decode_base64_string(credential)
            .ok_or_else(|| anyhow!("failed to decode shadowsocks credentials"))?;
        let (method, password) = decoded_credential
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid shadowsocks credentials"))?;
        (
            percent_decode_once_lossy(method),
            percent_decode_once_lossy(password),
        )
    };
    let parsed_host = Url::parse(&format!("http://{host_port}"))
        .context("invalid shadowsocks server endpoint")?;
    let host = parsed_host
        .host_str()
        .ok_or_else(|| anyhow!("shadowsocks host missing"))?
        .to_string();
    let port = parsed_host
        .port_or_known_default()
        .ok_or_else(|| anyhow!("shadowsocks port missing"))?;
    let display_name = display_name.unwrap_or_else(|| format!("{host}:{port}"));
    Ok(ShadowsocksShareLink {
        method,
        password,
        host,
        port,
        display_name,
    })
}

fn split_once_first(raw: &str, delimiter: char) -> (&str, Option<&str>) {
    if let Some((lhs, rhs)) = raw.split_once(delimiter) {
        (lhs, Some(rhs))
    } else {
        (raw, None)
    }
}

fn parse_alpn_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn deterministic_unit_f64(seed: u64) -> f64 {
    let mut value = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ceb9fe1a85ec53);
    value ^= value >> 33;
    (value as f64) / (u64::MAX as f64)
}
