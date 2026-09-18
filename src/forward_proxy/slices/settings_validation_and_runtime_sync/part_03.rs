#[derive(Default)]
struct BoundKeyRegistryBuilder {
    aliases: HashMap<String, String>,
    endpoint_keys: HashMap<String, String>,
    endpoint_bindings: HashMap<String, String>,
    descriptor_aliases: HashMap<String, HashSet<String>>,
    descriptors: Vec<ForwardProxyBindingNodeDescriptor>,
}

struct BoundKeyRegistry {
    aliases: HashMap<String, String>,
    endpoint_keys: HashMap<String, String>,
    endpoint_bindings: HashMap<String, String>,
    descriptors: Vec<ForwardProxyBindingNodeDescriptor>,
}

impl BoundKeyRegistryBuilder {
    fn add_endpoint(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        parts: &ForwardProxyBindingParts,
        name_counts: &HashMap<String, usize>,
        name_protocol_counts: &HashMap<(String, String), usize>,
    ) {
        let candidates = forward_proxy_binding_key_candidates(parts);
        let binding_key = choose_binding_key(parts, &candidates, name_counts, name_protocol_counts);
        let primary_endpoint = self
            .endpoint_keys
            .entry(binding_key.clone())
            .or_insert_with(|| endpoint.key.clone())
            .clone();
        self.endpoint_bindings
            .insert(endpoint.key.clone(), binding_key.clone());
        for alias in candidates
            .into_iter()
            .chain(self.endpoint_aliases(endpoint))
        {
            self.add_alias(binding_key.as_str(), alias);
        }
        if primary_endpoint == endpoint.key {
            self.descriptors.push(ForwardProxyBindingNodeDescriptor {
                key: binding_key,
                endpoint_key: endpoint.key.clone(),
                alias_keys: Vec::new(),
                source: endpoint.source.clone(),
                display_name: endpoint.display_name.clone(),
                protocol_label: endpoint.protocol.label().to_string(),
            });
        }
    }

    fn endpoint_aliases(&self, endpoint: &ForwardProxyEndpoint) -> Vec<String> {
        let mut aliases = vec![endpoint.key.clone()];
        if let Some(raw_url) = endpoint.raw_url.as_deref() {
            if let Some((canonical_key, storage_aliases)) = forward_proxy_storage_aliases(raw_url) {
                aliases.push(canonical_key);
                aliases.extend(storage_aliases);
            }
            aliases.extend(legacy_bound_proxy_key_aliases(raw_url, endpoint.protocol));
        }
        aliases
    }

    fn add_alias(&mut self, binding_key: &str, alias: String) {
        if alias == binding_key {
            return;
        }
        self.aliases
            .entry(alias.clone())
            .or_insert_with(|| binding_key.to_string());
        self.descriptor_aliases
            .entry(binding_key.to_string())
            .or_default()
            .insert(alias);
    }

    fn finish(mut self) -> BoundKeyRegistry {
        for descriptor in &mut self.descriptors {
            let mut aliases = self
                .descriptor_aliases
                .remove(&descriptor.key)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            aliases.sort();
            descriptor.alias_keys = aliases;
        }
        self.descriptors
            .sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));
        BoundKeyRegistry {
            aliases: self.aliases,
            endpoint_keys: self.endpoint_keys,
            endpoint_bindings: self.endpoint_bindings,
            descriptors: self.descriptors,
        }
    }
}

fn binding_name_counts(
    endpoint_parts: &[(ForwardProxyEndpoint, ForwardProxyBindingParts)],
) -> (HashMap<String, usize>, HashMap<(String, String), usize>) {
    let mut names = HashMap::new();
    let mut name_protocols = HashMap::new();
    for (_, parts) in endpoint_parts {
        *names.entry(parts.display_name.clone()).or_default() += 1;
        *name_protocols
            .entry((parts.display_name.clone(), parts.protocol_key.clone()))
            .or_default() += 1;
    }
    (names, name_protocols)
}

fn choose_binding_key(
    parts: &ForwardProxyBindingParts,
    candidates: &[String],
    name_counts: &HashMap<String, usize>,
    name_protocol_counts: &HashMap<(String, String), usize>,
) -> String {
    if name_counts
        .get(&parts.display_name)
        .copied()
        .unwrap_or_default()
        <= 1
    {
        candidates[0].clone()
    } else if name_protocol_counts
        .get(&(parts.display_name.clone(), parts.protocol_key.clone()))
        .copied()
        .unwrap_or_default()
        <= 1
    {
        candidates[1].clone()
    } else {
        candidates[2].clone()
    }
}

impl ForwardProxyProtocol {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Direct => "DIRECT",
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::Socks5 => "SOCKS5",
            Self::Socks5h => "SOCKS5H",
            Self::Vmess => "VMESS",
            Self::Vless => "VLESS",
            Self::Trojan => "TROJAN",
            Self::Shadowsocks => "SS",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyEndpoint {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol: ForwardProxyProtocol,
    pub(crate) endpoint_url: Option<Url>,
    pub(crate) raw_url: Option<String>,
}

impl ForwardProxyEndpoint {
    pub(crate) fn direct() -> Self {
        Self {
            key: FORWARD_PROXY_DIRECT_KEY.to_string(),
            source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
            display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
            protocol: ForwardProxyProtocol::Direct,
            endpoint_url: None,
            raw_url: None,
        }
    }

    pub(crate) fn is_selectable(&self) -> bool {
        self.endpoint_url.is_some()
    }

    pub(crate) fn is_bound_selectable(&self) -> bool {
        self.endpoint_url.is_some() || matches!(self.protocol, ForwardProxyProtocol::Direct)
    }

    pub(crate) fn requires_xray(&self) -> bool {
        matches!(
            self.protocol,
            ForwardProxyProtocol::Vmess
                | ForwardProxyProtocol::Vless
                | ForwardProxyProtocol::Trojan
                | ForwardProxyProtocol::Shadowsocks
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyRuntimeState {
    pub(crate) proxy_key: String,
    pub(crate) display_name: String,
    pub(crate) source: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) success_ema: f64,
    pub(crate) latency_ema_ms: Option<f64>,
    pub(crate) consecutive_failures: u32,
}

impl ForwardProxyRuntimeState {
    pub(crate) fn default_for_endpoint(
        endpoint: &ForwardProxyEndpoint,
        algo: ForwardProxyAlgo,
    ) -> Self {
        Self {
            proxy_key: endpoint.key.clone(),
            display_name: endpoint.display_name.clone(),
            source: endpoint.source.clone(),
            endpoint_url: endpoint.raw_url.clone(),
            weight: if endpoint.key == FORWARD_PROXY_DIRECT_KEY {
                match algo {
                    ForwardProxyAlgo::V1 => 1.0,
                    ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_DIRECT_INITIAL_WEIGHT,
                }
            } else {
                0.8
            },
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        }
    }

    pub(crate) fn is_penalized(&self) -> bool {
        self.weight <= 0.0
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyRuntimeRow {
    pub(crate) proxy_key: String,
    pub(crate) display_name: String,
    pub(crate) source: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) success_ema: f64,
    pub(crate) latency_ema_ms: Option<f64>,
    pub(crate) consecutive_failures: i64,
}

impl From<ForwardProxyRuntimeRow> for ForwardProxyRuntimeState {
    fn from(value: ForwardProxyRuntimeRow) -> Self {
        Self {
            proxy_key: value.proxy_key,
            display_name: value.display_name,
            source: value.source,
            endpoint_url: value.endpoint_url,
            weight: value.weight,
            success_ema: value.success_ema.clamp(0.0, 1.0),
            latency_ema_ms: value.latency_ema_ms,
            consecutive_failures: value.consecutive_failures.max(0) as u32,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyManager {
    pub(crate) algo: ForwardProxyAlgo,
    pub(crate) settings: ForwardProxySettings,
    pub(crate) endpoints: Vec<ForwardProxyEndpoint>,
    pub(crate) runtime: HashMap<String, ForwardProxyRuntimeState>,
    pub(crate) bound_key_aliases: HashMap<String, String>,
    pub(crate) bound_key_endpoint_keys: HashMap<String, String>,
    pub(crate) bound_key_by_endpoint_key: HashMap<String, String>,
    pub(crate) bound_node_descriptors: Vec<ForwardProxyBindingNodeDescriptor>,
    pub(crate) bound_group_runtime: HashMap<String, BoundForwardProxyGroupState>,
    pub(crate) selection_counter: u64,
    pub(crate) requests_since_probe: u64,
    pub(crate) probe_in_flight: bool,
    pub(crate) last_probe_at: DateTime<Utc>,
    pub(crate) last_subscription_refresh_at: Option<DateTime<Utc>>,
}

pub(crate) const BOUND_FORWARD_PROXY_SWITCH_FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Default)]
pub(crate) struct BoundForwardProxyGroupState {
    pub(crate) current_binding_key: Option<String>,
    pub(crate) consecutive_network_failures: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyBindingNodeDescriptor {
    pub(crate) key: String,
    pub(crate) endpoint_key: String,
    pub(crate) alias_keys: Vec<String>,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol_label: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ForwardProxyRouteScope {
    Automatic,
    PinnedProxyKey(String),
    BoundGroup {
        group_name: String,
        bound_proxy_keys: Vec<String>,
    },
    BoundProxyKeys {
        scope_key: String,
        bound_proxy_keys: Vec<String>,
    },
}

impl ForwardProxyRouteScope {
    pub(crate) fn from_group_binding(
        group_name: Option<&str>,
        bound_proxy_keys: Vec<String>,
    ) -> Self {
        let normalized_group_name = group_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let normalized_bound_proxy_keys = bound_proxy_keys
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| normalize_bound_proxy_key(&value).unwrap_or(value))
            .collect::<Vec<_>>();
        match (
            normalized_group_name,
            normalized_bound_proxy_keys.is_empty(),
        ) {
            (Some(group_name), false) => Self::BoundGroup {
                group_name,
                bound_proxy_keys: normalized_bound_proxy_keys,
            },
            _ => Self::Automatic,
        }
    }

    pub(crate) fn pinned(proxy_key: impl Into<String>) -> Self {
        Self::PinnedProxyKey(proxy_key.into())
    }

    pub(crate) fn bound_scope(scope_key: impl Into<String>, bound_proxy_keys: Vec<String>) -> Self {
        let scope_key = scope_key.into().trim().to_string();
        let normalized_bound_proxy_keys = bound_proxy_keys
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| normalize_bound_proxy_key(&value).unwrap_or(value))
            .collect::<Vec<_>>();
        if scope_key.is_empty() || normalized_bound_proxy_keys.is_empty() {
            Self::Automatic
        } else {
            Self::BoundProxyKeys {
                scope_key,
                bound_proxy_keys: normalized_bound_proxy_keys,
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardProxyRouteResultKind {
    CompletedRequest,
    NetworkFailure,
}
