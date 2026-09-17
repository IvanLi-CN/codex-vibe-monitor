include!("settings_validation_and_runtime_sync/part_01.rs");
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

impl ForwardProxyManager {
    #[cfg(test)]
    pub(crate) fn new(
        settings: ForwardProxySettings,
        runtime_rows: Vec<ForwardProxyRuntimeState>,
    ) -> Self {
        Self::with_algo(settings, runtime_rows, ForwardProxyAlgo::V1)
    }

    pub(crate) fn with_algo(
        settings: ForwardProxySettings,
        runtime_rows: Vec<ForwardProxyRuntimeState>,
        algo: ForwardProxyAlgo,
    ) -> Self {
        let runtime = runtime_rows
            .into_iter()
            .map(|mut entry| {
                Self::normalize_runtime_for_algo(&mut entry, algo);
                (entry.proxy_key.clone(), entry)
            })
            .collect::<HashMap<_, _>>();
        let mut manager = Self {
            algo,
            settings,
            endpoints: Vec::new(),
            runtime,
            bound_key_aliases: HashMap::new(),
            bound_key_endpoint_keys: HashMap::new(),
            bound_key_by_endpoint_key: HashMap::new(),
            bound_node_descriptors: Vec::new(),
            bound_group_runtime: HashMap::new(),
            selection_counter: 0,
            requests_since_probe: 0,
            probe_in_flight: false,
            last_probe_at: Utc::now() - ChronoDuration::seconds(algo.probe_interval_secs()),
            last_subscription_refresh_at: None,
        };
        manager.rebuild_endpoints(Vec::new());
        manager
    }

    pub(crate) fn normalize_runtime_for_algo(
        runtime: &mut ForwardProxyRuntimeState,
        algo: ForwardProxyAlgo,
    ) {
        runtime.success_ema = runtime.success_ema.clamp(0.0, 1.0);
        if runtime
            .latency_ema_ms
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            runtime.latency_ema_ms = None;
        }
        if !runtime.weight.is_finite() {
            runtime.weight = 0.0;
        }
        runtime.weight = match algo {
            ForwardProxyAlgo::V1 => runtime
                .weight
                .clamp(FORWARD_PROXY_WEIGHT_MIN, FORWARD_PROXY_WEIGHT_MAX),
            ForwardProxyAlgo::V2 => runtime
                .weight
                .clamp(FORWARD_PROXY_V2_WEIGHT_MIN, FORWARD_PROXY_V2_WEIGHT_MAX),
        };
    }

    pub(crate) fn apply_settings(&mut self, settings: ForwardProxySettings) {
        self.settings = settings;
        self.rebuild_endpoints(Vec::new());
    }

    pub(crate) fn apply_subscription_urls(&mut self, proxy_urls: Vec<String>) {
        let normalized_urls = normalize_proxy_url_entries(proxy_urls);
        let subscription_endpoints = normalize_proxy_endpoints_from_urls(
            &normalized_urls,
            FORWARD_PROXY_SOURCE_SUBSCRIPTION,
        );
        self.rebuild_endpoints(subscription_endpoints);
        self.last_subscription_refresh_at = Some(Utc::now());
    }

    pub(crate) fn rebuild_endpoints(&mut self, subscription_endpoints: Vec<ForwardProxyEndpoint>) {
        let mut merged = Vec::new();
        let manual = normalize_proxy_endpoints_from_urls(
            &self.settings.proxy_urls,
            FORWARD_PROXY_SOURCE_MANUAL,
        );
        let mut seen = HashSet::new();
        for endpoint in manual.into_iter().chain(subscription_endpoints) {
            if seen.insert(endpoint.key.clone()) {
                merged.push(endpoint);
            }
        }
        self.endpoints = merged;
        self.rebuild_bound_key_registry();

        let endpoint_snapshots = self.endpoints.clone();
        for endpoint in &endpoint_snapshots {
            self.migrate_runtime_aliases_to_endpoint(endpoint);
        }

        let algo = self.algo;
        for endpoint in &self.endpoints {
            match self.runtime.entry(endpoint.key.clone()) {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    let runtime = occupied.get_mut();
                    runtime.display_name = endpoint.display_name.clone();
                    runtime.source = endpoint.source.clone();
                    runtime.endpoint_url = endpoint.raw_url.clone();
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(ForwardProxyRuntimeState::default_for_endpoint(
                        endpoint, algo,
                    ));
                }
            }
        }
        self.ensure_non_zero_weight();
    }

    fn binding_parts_for_endpoint(
        endpoint: &ForwardProxyEndpoint,
    ) -> Option<ForwardProxyBindingParts> {
        endpoint.raw_url.as_deref().and_then(|raw_url| {
            forward_proxy_binding_parts_from_raw(raw_url, Some(&endpoint.display_name))
        })
    }

    fn rebuild_bound_key_registry(&mut self) {
        let mut endpoint_parts = self
            .endpoints
            .iter()
            .filter_map(|endpoint| {
                Self::binding_parts_for_endpoint(endpoint).map(|parts| (endpoint.clone(), parts))
            })
            .collect::<Vec<_>>();
        endpoint_parts.sort_by(|lhs, rhs| lhs.0.key.cmp(&rhs.0.key));
        let (name_counts, name_protocol_counts) = binding_name_counts(&endpoint_parts);
        let mut builder = BoundKeyRegistryBuilder::default();
        for (endpoint, parts) in endpoint_parts {
            builder.add_endpoint(&endpoint, &parts, &name_counts, &name_protocol_counts);
        }
        let registry = builder.finish();
        self.bound_key_aliases = registry.aliases;
        self.bound_key_endpoint_keys = registry.endpoint_keys;
        self.bound_key_by_endpoint_key = registry.endpoint_bindings;
        self.bound_node_descriptors = registry.descriptors;
    }

    fn resolve_current_bound_proxy_key_from_parts(
        &self,
        parts: &ForwardProxyBindingParts,
    ) -> Option<String> {
        for candidate in forward_proxy_binding_key_candidates(parts) {
            if self.bound_key_endpoint_keys.contains_key(&candidate) {
                return Some(candidate);
            }
            if let Some(canonical) = self.bound_key_aliases.get(&candidate) {
                return Some(canonical.clone());
            }
        }
        None
    }

    fn resolve_current_bound_proxy_key(&self, proxy_key: &str) -> Option<String> {
        let normalized = normalize_bound_proxy_key(proxy_key)?;
        if normalized == FORWARD_PROXY_DIRECT_KEY {
            return Some(normalized);
        }
        if self.bound_key_endpoint_keys.contains_key(&normalized) {
            return Some(normalized);
        }
        self.bound_key_aliases.get(&normalized).cloned()
    }

    fn resolve_bound_proxy_key_from_history_row(
        &self,
        row: &ForwardProxyMetadataHistoryRow,
    ) -> Option<String> {
        let raw = row.endpoint_url.as_deref()?;
        let parts = forward_proxy_binding_parts_from_raw(raw, Some(&row.display_name))?;
        self.resolve_current_bound_proxy_key_from_parts(&parts)
    }

    pub(crate) fn resolve_current_or_historical_bound_proxy_key(
        &self,
        proxy_key: &str,
        history_row: Option<&ForwardProxyMetadataHistoryRow>,
    ) -> Option<String> {
        self.resolve_current_bound_proxy_key(proxy_key).or_else(|| {
            history_row.and_then(|row| self.resolve_bound_proxy_key_from_history_row(row))
        })
    }

    pub(crate) fn canonicalize_bound_proxy_key(
        &self,
        proxy_key: &str,
        history_row: Option<&ForwardProxyMetadataHistoryRow>,
    ) -> Option<String> {
        let normalized = normalize_bound_proxy_key(proxy_key)?;
        self.resolve_current_or_historical_bound_proxy_key(&normalized, history_row)
            .or(Some(normalized))
    }

    pub(crate) fn current_bound_group_binding_key(
        &self,
        group_name: &str,
        bound_proxy_keys: &[String],
    ) -> Option<String> {
        self.current_bound_scope_binding_key(group_name, bound_proxy_keys)
    }

    pub(crate) fn current_bound_scope_binding_key(
        &self,
        scope_key: &str,
        bound_proxy_keys: &[String],
    ) -> Option<String> {
        let available_keys = self.selectable_bound_proxy_keys_in_order(bound_proxy_keys);
        if available_keys.is_empty() {
            return None;
        }
        self.bound_group_runtime
            .get(scope_key)
            .and_then(|state| state.current_binding_key.as_deref())
            .and_then(|key| {
                self.resolve_current_bound_proxy_key(key)
                    .or_else(|| normalize_bound_proxy_key(key))
            })
            .filter(|key| available_keys.contains(key))
    }

    fn migrate_runtime_aliases_to_endpoint(&mut self, endpoint: &ForwardProxyEndpoint) {
        let Some(raw_url) = endpoint.raw_url.as_deref() else {
            return;
        };
        let Some((canonical_key, aliases)) = forward_proxy_storage_aliases(raw_url) else {
            return;
        };
        if canonical_key != endpoint.key {
            return;
        }

        if self.runtime.contains_key(&endpoint.key) {
            for alias in aliases {
                self.runtime.remove(&alias);
            }
            return;
        }

        let mut migrated = None;
        for alias in &aliases {
            if let Some(runtime) = self.runtime.remove(alias) {
                migrated = Some(runtime);
                break;
            }
        }
        for alias in aliases {
            self.runtime.remove(&alias);
        }
        if let Some(mut runtime) = migrated {
            runtime.proxy_key = endpoint.key.clone();
            runtime.endpoint_url = Some(raw_url.to_string());
            self.runtime.insert(endpoint.key.clone(), runtime);
        }
    }

    pub(crate) fn ensure_non_zero_weight(&mut self) {
        let minimum = match self.algo {
            ForwardProxyAlgo::V1 => 1,
            ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_MIN_POSITIVE_CANDIDATES,
        };
        self.ensure_min_positive_candidates(minimum, self.algo.probe_recovery_weight());
    }

    pub(crate) fn selectable_endpoint_keys(&self) -> HashSet<&str> {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.is_selectable())
            .map(|endpoint| endpoint.key.as_str())
            .collect::<HashSet<_>>()
    }

    pub(crate) fn selectable_proxy_keys(&self) -> HashSet<String> {
        let mut keys = self
            .bound_node_descriptors
            .iter()
            .filter_map(|descriptor| {
                self.endpoints
                    .iter()
                    .find(|endpoint| endpoint.key == descriptor.endpoint_key)
                    .filter(|endpoint| endpoint.is_bound_selectable())
                    .map(|_| descriptor.key.clone())
            })
            .collect::<HashSet<_>>();
        keys.insert(FORWARD_PROXY_DIRECT_KEY.to_string());
        keys
    }

    pub(crate) fn ensure_min_positive_candidates(&mut self, minimum: usize, recovery_weight: f64) {
        if minimum == 0 {
            return;
        }

        let selectable_keys = self.selectable_endpoint_keys();
        let active_keys = if selectable_keys.is_empty() {
            self.endpoints
                .iter()
                .map(|endpoint| endpoint.key.as_str())
                .collect::<HashSet<_>>()
        } else {
            selectable_keys
        };
        let mut positive_count = self
            .runtime
            .values()
            .filter(|entry| {
                active_keys.contains(entry.proxy_key.as_str())
                    && entry.weight > 0.0
                    && entry.weight.is_finite()
            })
            .count();
        if positive_count >= minimum {
            return;
        }

        let mut candidates = self
            .runtime
            .values()
            .filter(|entry| active_keys.contains(entry.proxy_key.as_str()))
            .map(|entry| (entry.proxy_key.clone(), entry.weight))
            .collect::<Vec<_>>();
        candidates.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));

        for (proxy_key, _) in candidates {
            if positive_count >= minimum {
                break;
            }
            if let Some(entry) = self.runtime.get_mut(&proxy_key)
                && !(entry.weight > 0.0 && entry.weight.is_finite())
            {
                entry.weight = recovery_weight;
                if self.algo == ForwardProxyAlgo::V2 {
                    entry.consecutive_failures = 0;
                }
                positive_count += 1;
            }
        }
    }

    pub(crate) fn snapshot_runtime(&self) -> Vec<ForwardProxyRuntimeState> {
        self.endpoints
            .iter()
            .filter_map(|endpoint| self.runtime.get(&endpoint.key).cloned())
            .collect()
    }

    fn next_random_index(&mut self, upper_bound: usize) -> usize {
        debug_assert!(upper_bound > 0);
        self.selection_counter = self.selection_counter.wrapping_add(1);
        let random = deterministic_unit_f64(self.selection_counter);
        ((random * upper_bound as f64).floor() as usize).min(upper_bound.saturating_sub(1))
    }

    fn selectable_bound_proxy_keys(&self, bound_proxy_keys: &[String]) -> Vec<String> {
        let selectable = self.selectable_proxy_keys();
        let mut seen = HashSet::new();
        let mut available = Vec::new();
        for key in bound_proxy_keys {
            let normalized = key.trim();
            if normalized.is_empty() {
                continue;
            }
            let canonical = self
                .resolve_current_bound_proxy_key(normalized)
                .unwrap_or_else(|| normalized.to_string());
            if !selectable.contains(&canonical) || !seen.insert(canonical.clone()) {
                continue;
            }
            available.push(canonical);
        }
        available.sort();
        available
    }

    pub(crate) fn selectable_bound_proxy_keys_in_order(
        &self,
        bound_proxy_keys: &[String],
    ) -> Vec<String> {
        let selectable = self.selectable_proxy_keys();
        let mut seen = HashSet::new();
        let mut available = Vec::new();
        for key in bound_proxy_keys {
            let normalized = key.trim();
            if normalized.is_empty() {
                continue;
            }
            let canonical = normalize_bound_proxy_key(normalized)
                .map(|value| self.bound_key_aliases.get(&value).cloned().unwrap_or(value))
                .unwrap_or_else(|| normalized.to_string());
            if !selectable.contains(&canonical) || !seen.insert(canonical.clone()) {
                continue;
            }
            available.push(canonical);
        }
        available
    }

    pub(crate) fn has_selectable_bound_proxy_keys(&self, bound_proxy_keys: &[String]) -> bool {
        !self
            .selectable_bound_proxy_keys(bound_proxy_keys)
            .is_empty()
    }

    fn bound_proxy_key_effective_weight(&self, binding_key: &str) -> f64 {
        if binding_key == FORWARD_PROXY_DIRECT_KEY {
            return 0.0;
        }
        let Some(endpoint_key) = self.bound_key_endpoint_keys.get(binding_key) else {
            return 0.0;
        };
        let Some(runtime) = self.runtime.get(endpoint_key) else {
            return 0.0;
        };
        if !(runtime.weight > 0.0 && runtime.weight.is_finite()) {
            return 0.0;
        }
        if self.algo == ForwardProxyAlgo::V2 {
            let success_factor = runtime.success_ema.clamp(0.0, 1.0).powi(8).max(0.01);
            runtime.weight.powi(2) * success_factor
        } else {
            runtime.weight
        }
    }

    fn choose_best_bound_proxy_key(
        &self,
        available_keys: &[String],
        exclude_key: Option<&str>,
    ) -> Option<String> {
        available_keys
            .iter()
            .filter(|candidate| Some(candidate.as_str()) != exclude_key)
            .cloned()
            .max_by(|lhs, rhs| {
                self.bound_proxy_key_effective_weight(lhs)
                    .total_cmp(&self.bound_proxy_key_effective_weight(rhs))
                    .then_with(|| rhs.cmp(lhs))
            })
    }

    pub(crate) fn select_auto_proxy(&mut self) -> Option<SelectedForwardProxy> {
        self.selection_counter = self.selection_counter.wrapping_add(1);
        self.requests_since_probe = self.requests_since_probe.saturating_add(1);
        self.ensure_non_zero_weight();

        let mut candidates = Vec::new();
        let mut total_weight = 0.0f64;
        for endpoint in &self.endpoints {
            if !endpoint.is_selectable() {
                continue;
            }
            if let Some(runtime) = self.runtime.get(&endpoint.key)
                && runtime.weight > 0.0
                && runtime.weight.is_finite()
            {
                let effective_weight = if self.algo == ForwardProxyAlgo::V2 {
                    let success_factor = runtime.success_ema.clamp(0.0, 1.0).powi(8).max(0.01);
                    runtime.weight.powi(2) * success_factor
                } else {
                    runtime.weight
                };
                total_weight += effective_weight;
                candidates.push((endpoint, effective_weight));
            }
        }

        if self.algo == ForwardProxyAlgo::V2 && candidates.len() > 3 {
            candidates.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));
            candidates.truncate(3);
            total_weight = candidates.iter().map(|(_, weight)| *weight).sum::<f64>();
        }

        if candidates.is_empty() {
            return None;
        }

        let seed = self.selection_counter;
        let random = deterministic_unit_f64(seed);
        let mut threshold = random * total_weight;
        let mut last_candidate: Option<&ForwardProxyEndpoint> = None;
        for (endpoint, weight) in candidates {
            last_candidate = Some(endpoint);
            if threshold <= weight {
                return Some(SelectedForwardProxy::from_endpoint(endpoint));
            }
            threshold -= weight;
        }
        last_candidate.map(SelectedForwardProxy::from_endpoint)
    }

    fn select_bound_scope_proxy(
        &mut self,
        scope_key: &str,
        bound_proxy_keys: &[String],
    ) -> Result<SelectedForwardProxy> {
        let available_keys = self.selectable_bound_proxy_keys(bound_proxy_keys);
        if available_keys.is_empty() {
            self.bound_group_runtime.remove(scope_key);
            bail!("bound forward proxy group has no selectable nodes");
        }
        let existing_current = self
            .bound_group_runtime
            .get(scope_key)
            .and_then(|state| state.current_binding_key.clone())
            .filter(|key| available_keys.contains(key));
        let selected_binding_key = existing_current.unwrap_or_else(|| {
            self.choose_best_bound_proxy_key(&available_keys, None)
                .expect("available bound proxy keys should not be empty")
        });
        let state = self
            .bound_group_runtime
            .entry(scope_key.to_string())
            .or_default();
        state.current_binding_key = Some(selected_binding_key.clone());
        if selected_binding_key == FORWARD_PROXY_DIRECT_KEY {
            return Ok(SelectedForwardProxy::from_endpoint(
                &ForwardProxyEndpoint::direct(),
            ));
        }
        let endpoint_key = self
            .bound_key_endpoint_keys
            .get(&selected_binding_key)
            .ok_or_else(|| anyhow!("selected bound proxy disappeared from runtime"))?;
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key == *endpoint_key)
            .ok_or_else(|| anyhow!("selected bound proxy disappeared from runtime"))?;
        Ok(SelectedForwardProxy::from_endpoint(endpoint))
    }

    fn select_pinned_proxy_key(&self, proxy_key: &str) -> Result<SelectedForwardProxy> {
        let normalized_proxy_key = proxy_key.trim();
        if normalized_proxy_key.is_empty() {
            bail!("pinned forward proxy key is empty");
        }
        if normalized_proxy_key == FORWARD_PROXY_DIRECT_KEY {
            return Ok(SelectedForwardProxy::from_endpoint(
                &ForwardProxyEndpoint::direct(),
            ));
        }
        let canonical_proxy_key = self
            .resolve_current_bound_proxy_key(normalized_proxy_key)
            .unwrap_or_else(|| normalized_proxy_key.to_string());
        let endpoint_key = self
            .bound_key_endpoint_keys
            .get(&canonical_proxy_key)
            .cloned()
            .unwrap_or(canonical_proxy_key);
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key == endpoint_key)
            .ok_or_else(|| anyhow!("pinned forward proxy key is no longer available"))?;
        if !endpoint.is_bound_selectable() {
            bail!("pinned forward proxy key is no longer available");
        }
        Ok(SelectedForwardProxy::from_endpoint(endpoint))
    }

    pub(crate) fn select_proxy_for_scope(
        &mut self,
        scope: &ForwardProxyRouteScope,
    ) -> Result<SelectedForwardProxy> {
        match scope {
            ForwardProxyRouteScope::Automatic => {
                if let Some(selected) = self.select_auto_proxy() {
                    Ok(selected)
                } else {
                    #[cfg(test)]
                    {
                        Ok(SelectedForwardProxy::from_endpoint(
                            &ForwardProxyEndpoint::direct(),
                        ))
                    }
                    #[cfg(not(test))]
                    {
                        Err(anyhow!("no selectable forward proxy nodes configured"))
                    }
                }
            }
            ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => {
                self.select_pinned_proxy_key(proxy_key)
            }
            ForwardProxyRouteScope::BoundGroup {
                group_name,
                bound_proxy_keys,
            } => self.select_bound_scope_proxy(group_name, bound_proxy_keys),
            ForwardProxyRouteScope::BoundProxyKeys {
                scope_key,
                bound_proxy_keys,
            } => self.select_bound_scope_proxy(scope_key, bound_proxy_keys),
        }
    }

    pub(crate) fn record_scope_result(
        &mut self,
        scope: &ForwardProxyRouteScope,
        selected_proxy_key: &str,
        result: ForwardProxyRouteResultKind,
    ) {
        let (scope_key, bound_proxy_keys) = match scope {
            ForwardProxyRouteScope::BoundGroup {
                group_name,
                bound_proxy_keys,
            } => (group_name, bound_proxy_keys),
            ForwardProxyRouteScope::BoundProxyKeys {
                scope_key,
                bound_proxy_keys,
            } => (scope_key, bound_proxy_keys),
            _ => return,
        };
        let available_keys = self.selectable_bound_proxy_keys(bound_proxy_keys);
        if available_keys.is_empty() {
            self.bound_group_runtime.remove(scope_key);
            return;
        }

        let selected_binding_key = self
            .resolve_current_bound_proxy_key(selected_proxy_key)
            .unwrap_or_else(|| selected_proxy_key.to_string());
        let mut should_switch = false;
        {
            let state = self
                .bound_group_runtime
                .entry(scope_key.clone())
                .or_default();
            state.current_binding_key = Some(selected_binding_key.clone());
            match result {
                ForwardProxyRouteResultKind::CompletedRequest => {
                    state.consecutive_network_failures = 0;
                }
                ForwardProxyRouteResultKind::NetworkFailure => {
                    state.consecutive_network_failures =
                        state.consecutive_network_failures.saturating_add(1);
                    should_switch = state.consecutive_network_failures
                        >= BOUND_FORWARD_PROXY_SWITCH_FAILURE_THRESHOLD;
                }
            }
        }

        if should_switch
            && let Some(next_binding_key) = self
                .choose_best_bound_proxy_key(&available_keys, Some(selected_binding_key.as_str()))
        {
            let state = self
                .bound_group_runtime
                .entry(scope_key.clone())
                .or_default();
            state.current_binding_key = Some(next_binding_key);
            state.consecutive_network_failures = 0;
        }
    }

    pub(crate) fn binding_nodes(&self) -> Vec<ForwardProxyBindingNodeResponse> {
        let mut nodes = self
            .bound_node_descriptors
            .iter()
            .map(|descriptor| {
                let penalized = self
                    .runtime
                    .get(&descriptor.endpoint_key)
                    .is_some_and(ForwardProxyRuntimeState::is_penalized);
                ForwardProxyBindingNodeResponse {
                    key: descriptor.key.clone(),
                    alias_keys: descriptor.alias_keys.clone(),
                    source: descriptor.source.clone(),
                    display_name: descriptor.display_name.clone(),
                    protocol_label: descriptor.protocol_label.clone(),
                    egress_ip: None,
                    egress_ip_checked_at: None,
                    egress_ip_provider: None,
                    egress_ip_error: None,
                    egress_ip_error_at: None,
                    penalized,
                    selectable: self
                        .endpoints
                        .iter()
                        .find(|endpoint| endpoint.key == descriptor.endpoint_key)
                        .is_some_and(ForwardProxyEndpoint::is_bound_selectable),
                    last24h: Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        nodes.push(ForwardProxyBindingNodeResponse {
            key: FORWARD_PROXY_DIRECT_KEY.to_string(),
            alias_keys: Vec::new(),
            source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
            display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
            protocol_label: ForwardProxyProtocol::Direct.label().to_string(),
            egress_ip: None,
            egress_ip_checked_at: None,
            egress_ip_provider: None,
            egress_ip_error: None,
            egress_ip_error_at: None,
            penalized: false,
            selectable: true,
            last24h: Vec::new(),
        });
        nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));
        nodes
    }

    pub(crate) fn record_attempt(
        &mut self,
        proxy_key: &str,
        success: bool,
        latency_ms: Option<f64>,
        is_probe: bool,
    ) {
        if !self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.key == proxy_key)
        {
            return;
        }
        let Some(runtime) = self.runtime.get_mut(proxy_key) else {
            return;
        };

        Self::update_runtime_ema(runtime, success, latency_ms);
        match self.algo {
            ForwardProxyAlgo::V1 => Self::record_attempt_v1(runtime, success, is_probe),
            ForwardProxyAlgo::V2 => Self::record_attempt_v2(runtime, success, is_probe),
        }
        self.ensure_non_zero_weight();
    }

    pub(crate) fn update_runtime_ema(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        latency_ms: Option<f64>,
    ) {
        runtime.success_ema = runtime.success_ema * 0.9 + if success { 0.1 } else { 0.0 };
        if let Some(latency_ms) = latency_ms.filter(|value| value.is_finite() && *value >= 0.0) {
            runtime.latency_ema_ms = Some(match runtime.latency_ema_ms {
                Some(previous) => previous * 0.8 + latency_ms * 0.2,
                None => latency_ms,
            });
        }
    }

    pub(crate) fn record_attempt_v1(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        is_probe: bool,
    ) {
        if success {
            runtime.consecutive_failures = 0;
            let latency_penalty = runtime
                .latency_ema_ms
                .map(|value| (value / 2500.0).min(0.6))
                .unwrap_or(0.0);
            runtime.weight += FORWARD_PROXY_WEIGHT_SUCCESS_BONUS - latency_penalty;
            if is_probe && runtime.weight <= 0.0 {
                runtime.weight = FORWARD_PROXY_PROBE_RECOVERY_WEIGHT;
            }
        } else {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let failure_penalty = FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_BASE
                + f64::from(runtime.consecutive_failures.saturating_sub(1))
                    * FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_STEP;
            runtime.weight -= failure_penalty;
        }

        runtime.weight = runtime
            .weight
            .clamp(FORWARD_PROXY_WEIGHT_MIN, FORWARD_PROXY_WEIGHT_MAX);

        if success && runtime.weight < FORWARD_PROXY_WEIGHT_RECOVERY {
            runtime.weight = runtime.weight.max(FORWARD_PROXY_WEIGHT_RECOVERY * 0.5);
        }
    }

    pub(crate) fn record_attempt_v2(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        is_probe: bool,
    ) {
        if success {
            runtime.consecutive_failures = 0;
            let latency_penalty = runtime
                .latency_ema_ms
                .map(|value| {
                    (value / FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_DIVISOR)
                        .min(FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_CAP)
                })
                .unwrap_or(0.0);
            let success_gain = (FORWARD_PROXY_V2_WEIGHT_SUCCESS_BASE - latency_penalty)
                .max(FORWARD_PROXY_V2_WEIGHT_SUCCESS_MIN_GAIN);
            runtime.weight += success_gain;
            if is_probe && runtime.weight <= 0.0 {
                runtime.weight = FORWARD_PROXY_V2_PROBE_RECOVERY_WEIGHT;
            }
        } else {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let failure_penalty = (FORWARD_PROXY_V2_WEIGHT_FAILURE_BASE
                + f64::from(runtime.consecutive_failures) * FORWARD_PROXY_V2_WEIGHT_FAILURE_STEP)
                .min(FORWARD_PROXY_V2_WEIGHT_FAILURE_MAX);
            runtime.weight -= failure_penalty;
        }

        runtime.weight = runtime
            .weight
            .clamp(FORWARD_PROXY_V2_WEIGHT_MIN, FORWARD_PROXY_V2_WEIGHT_MAX);

        if success && runtime.weight < FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR {
            runtime.weight = FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR;
        }
    }

    pub(crate) fn should_probe_penalized_proxy(&self) -> bool {
        let selectable_keys = self.selectable_endpoint_keys();
        if selectable_keys.is_empty() {
            return false;
        }
        let has_penalized = self.runtime.values().any(|entry| {
            selectable_keys.contains(entry.proxy_key.as_str()) && entry.is_penalized()
        });
        if !has_penalized || self.probe_in_flight {
            return false;
        }
        self.requests_since_probe >= self.algo.probe_every_requests()
            || (Utc::now() - self.last_probe_at).num_seconds() >= self.algo.probe_interval_secs()
    }

    pub(crate) fn mark_probe_started(&mut self) -> Option<SelectedForwardProxy> {
        if !self.should_probe_penalized_proxy() {
            return None;
        }
        let selectable_keys = self.selectable_endpoint_keys();
        let selected = self
            .runtime
            .values()
            .filter(|entry| {
                entry.is_penalized() && selectable_keys.contains(entry.proxy_key.as_str())
            })
            .max_by(|lhs, rhs| lhs.weight.total_cmp(&rhs.weight))
            .and_then(|entry| {
                self.endpoints
                    .iter()
                    .find(|item| item.key == entry.proxy_key)
            })
            .cloned()?;
        self.probe_in_flight = true;
        self.requests_since_probe = 0;
        self.last_probe_at = Utc::now();
        Some(SelectedForwardProxy::from_endpoint(&selected))
    }

    pub(crate) fn mark_probe_finished(&mut self) {
        self.probe_in_flight = false;
        self.last_probe_at = Utc::now();
    }
}
