impl PricingSettingsUpdateRequest {
    pub(crate) fn normalized(self) -> Result<PricingCatalog, (StatusCode, String)> {
        let version = normalize_pricing_catalog_version(self.catalog_version).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "catalogVersion must be a non-empty string".to_string(),
            )
        })?;
        let mut models = HashMap::new();
        for entry in self.entries {
            let model_id = entry.model.trim();
            if model_id.is_empty() || model_id.len() > 128 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid model id: {}", entry.model),
                ));
            }
            if !entry.input_per_1m.is_finite()
                || !entry.output_per_1m.is_finite()
                || entry.input_per_1m < 0.0
                || entry.output_per_1m < 0.0
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid pricing values for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_input_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheInputPer1m for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_read_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheReadPer1m for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_write_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheWritePer1m for model: {model_id}"),
                ));
            }
            if let Some(reasoning) = entry.reasoning_per_1m
                && (!reasoning.is_finite() || reasoning < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid reasoningPer1m for model: {model_id}"),
                ));
            }

            let cache_read_per_1m = entry.cache_read_per_1m.or(entry.cache_input_per_1m);
            let inserted = models.insert(
                model_id.to_string(),
                ModelPricing {
                    input_per_1m: entry.input_per_1m,
                    output_per_1m: entry.output_per_1m,
                    cache_input_per_1m: cache_read_per_1m,
                    cache_read_per_1m,
                    cache_write_per_1m: entry.cache_write_per_1m,
                    reasoning_per_1m: entry.reasoning_per_1m,
                    source: normalize_pricing_source(entry.source),
                },
            );
            if inserted.is_some() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("duplicate model id: {model_id}"),
                ));
            }
        }
        Ok(PricingCatalog { version, models })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingSettingsResponse {
    pub(crate) catalog_version: String,
    pub(crate) entries: Vec<PricingEntry>,
}

impl PricingSettingsResponse {
    pub(crate) fn from_catalog(catalog: &PricingCatalog) -> Self {
        let mut entries = catalog
            .models
            .iter()
            .map(|(model, pricing)| {
                let cache_read_per_1m = pricing.effective_cache_read_per_1m();
                PricingEntry {
                    model: model.clone(),
                    input_per_1m: pricing.input_per_1m,
                    output_per_1m: pricing.output_per_1m,
                    cache_input_per_1m: cache_read_per_1m,
                    cache_read_per_1m,
                    cache_write_per_1m: pricing.cache_write_per_1m,
                    reasoning_per_1m: pricing.reasoning_per_1m,
                    source: pricing.source.clone(),
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.model.cmp(&b.model));
        Self {
            catalog_version: catalog.version.clone(),
            entries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettings {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) websocket_enabled: bool,
    pub(crate) upstream_websocket_default_enabled: bool,
    pub(crate) request_body_logging_enabled: bool,
    pub(crate) response_body_logging_enabled: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
    pub(crate) enabled_preset_models: Vec<String>,
}

pub(crate) fn normalize_proxy_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

pub(crate) fn decode_proxy_upstream_429_max_retries(raw: Option<i64>) -> u8 {
    raw.and_then(|value| u8::try_from(value).ok())
        .map(normalize_proxy_upstream_429_max_retries)
        .unwrap_or(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES)
}

impl Default for ProxyModelSettings {
    fn default() -> Self {
        Self {
            hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            merge_upstream_enabled: DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            websocket_enabled: DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
            upstream_websocket_default_enabled:
                DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
            request_body_logging_enabled: true,
            response_body_logging_enabled: true,
            encrypted_session_owner_routing_enabled:
                DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
            enabled_preset_models: default_enabled_preset_models(),
        }
    }
}

impl ProxyModelSettings {
    pub(crate) fn normalized(self) -> Self {
        let merge_upstream_enabled = if self.hijack_enabled {
            self.merge_upstream_enabled
        } else {
            false
        };
        Self {
            hijack_enabled: self.hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries: normalize_proxy_upstream_429_max_retries(
                self.upstream_429_max_retries,
            ),
            websocket_enabled: self.websocket_enabled,
            upstream_websocket_default_enabled: self.upstream_websocket_default_enabled,
            request_body_logging_enabled: self.request_body_logging_enabled,
            response_body_logging_enabled: self.response_body_logging_enabled,
            encrypted_session_owner_routing_enabled: self.encrypted_session_owner_routing_enabled,
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyModelSettingsRow {
    pub(crate) hijack_enabled: i64,
    pub(crate) merge_upstream_enabled: i64,
    pub(crate) upstream_429_max_retries: Option<i64>,
    pub(crate) openai_proxy_websocket_enabled: Option<i64>,
    pub(crate) openai_proxy_upstream_websocket_default_enabled: Option<i64>,
    pub(crate) request_body_logging_enabled: Option<i64>,
    pub(crate) response_body_logging_enabled: Option<i64>,
    pub(crate) encrypted_session_owner_routing_enabled: Option<i64>,
    pub(crate) enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
            upstream_429_max_retries: decode_proxy_upstream_429_max_retries(
                value.upstream_429_max_retries,
            ),
            websocket_enabled: value.openai_proxy_websocket_enabled.unwrap_or(0) != 0,
            upstream_websocket_default_enabled: value
                .openai_proxy_upstream_websocket_default_enabled
                .unwrap_or(0)
                != 0,
            request_body_logging_enabled: value.request_body_logging_enabled.unwrap_or(1) != 0,
            response_body_logging_enabled: value.response_body_logging_enabled.unwrap_or(1) != 0,
            encrypted_session_owner_routing_enabled: value
                .encrypted_session_owner_routing_enabled
                .unwrap_or(DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED as i64)
                != 0,
            enabled_preset_models: decode_enabled_preset_models(
                value.enabled_preset_models_json.as_deref(),
            ),
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettingsUpdateRequest {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    #[serde(default)]
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    #[serde(default)]
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default)]
    pub(crate) websocket_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_websocket_default_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) request_body_logging_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) response_body_logging_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) encrypted_session_owner_routing_enabled: Option<bool>,
    #[serde(default = "default_enabled_preset_models")]
    pub(crate) enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettingsResponse {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) websocket_enabled: bool,
    pub(crate) upstream_websocket_default_enabled: bool,
    pub(crate) request_body_logging_enabled: bool,
    pub(crate) response_body_logging_enabled: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
    pub(crate) default_hijack_enabled: bool,
    pub(crate) models: Vec<String>,
    pub(crate) image_models: Vec<String>,
    pub(crate) enabled_models: Vec<String>,
}

impl ProxyModelSettingsResponse {
    pub(crate) fn from_settings(value: ProxyModelSettings) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled,
            merge_upstream_enabled: value.merge_upstream_enabled,
            fast_mode_rewrite_mode: "disabled".to_string(),
            upstream_429_max_retries: value.upstream_429_max_retries,
            websocket_enabled: value.websocket_enabled,
            upstream_websocket_default_enabled: value.upstream_websocket_default_enabled,
            request_body_logging_enabled: value.request_body_logging_enabled,
            response_body_logging_enabled: value.response_body_logging_enabled,
            encrypted_session_owner_routing_enabled: value.encrypted_session_owner_routing_enabled,
            default_hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            models: PROXY_PRESET_MODEL_IDS
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            image_models: PROXY_IMAGE_MODEL_IDS
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            enabled_models: value.enabled_preset_models,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsResponse {
    pub(crate) proxy: ProxyModelSettingsResponse,
    pub(crate) forward_proxy: ForwardProxySettingsResponse,
    pub(crate) pricing: PricingSettingsResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemTaskKind {
    RetentionArchive,
    StartupBackfill,
    HourlyRollupBootstrap,
    ForwardProxySubscriptionRefresh,
}

impl SystemTaskKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RetentionArchive => "retention_archive",
            Self::StartupBackfill => "startup_backfill",
            Self::HourlyRollupBootstrap => "hourly_rollup_bootstrap",
            Self::ForwardProxySubscriptionRefresh => "forward_proxy_subscription_refresh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemTaskStatus {
    Running,
    Success,
    Failed,
    Skipped,
}

impl SystemTaskStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SystemStatusCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) response: SystemStatusResponse,
    // The durable inventory value captured by the background refresh. An in-memory override
    // may temporarily replace the response state, then restore this value without request I/O.
    pub(crate) raw_metrics_inventory_state: String,
}

#[derive(Debug, Default)]
pub(crate) struct SystemStatusCacheState {
    pub(crate) latest: Option<SystemStatusCacheEntry>,
    pub(crate) in_flight: Option<watch::Sender<bool>>,
    // This admission survives a logically cancelled refresh until its blocking filesystem scan
    // actually exits. A started `spawn_blocking` filesystem call cannot be cancelled safely.
    pub(crate) filesystem_scan_in_flight: Arc<AtomicBool>,
    pub(crate) waiter_count: usize,
    pub(crate) raw_metrics_health_override: Option<String>,
}

pub(crate) fn default_enabled_preset_models() -> Vec<String> {
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| **model != "gpt-5.4-mini")
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn normalize_enabled_preset_models(enabled_models: Vec<String>) -> Vec<String> {
    let enabled_set: HashSet<&str> = enabled_models.iter().map(String::as_str).collect();
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| enabled_set.contains(**model))
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn decode_enabled_preset_models(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized)
            .map(normalize_enabled_preset_models)
            .unwrap_or_else(|_| default_enabled_preset_models()),
        None => default_enabled_preset_models(),
    }
}

pub(crate) fn default_pricing_source_custom() -> String {
    "custom".to_string()
}

pub(crate) fn normalize_pricing_catalog_version(raw: String) -> Option<String> {
    let normalized = raw.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) fn normalize_pricing_source(raw: String) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_pricing_source_custom()
    } else {
        normalized
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HttpClients {
    pub(crate) shared: Client,
    pub(crate) pool_upstream: Client,
    pub(crate) proxy: Client,
    pub(crate) timeout: Duration,
    pub(crate) user_agent: String,
}

impl HttpClients {
    pub(crate) fn build(config: &AppConfig) -> Result<Self> {
        let timeout = config.request_timeout;
        let user_agent = config.user_agent.clone();

        let shared = Self::builder(Some(timeout), &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct shared HTTP client")?;

        // Pool live upstream traffic can legitimately stream well past REQUEST_TIMEOUT_SECS.
        // Handshake and upload budgets are enforced by route-specific timeout wrappers instead.
        let pool_upstream = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct pool upstream HTTP client")?;

        let proxy = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to construct proxy HTTP client")?;

        Ok(Self {
            shared,
            pool_upstream,
            proxy,
            timeout,
            user_agent,
        })
    }

    pub(crate) fn client_for_parallelism(&self, force_new_connection: bool) -> Result<Client> {
        if force_new_connection {
            let client = Self::builder(Some(self.timeout), &self.user_agent)
                .pool_max_idle_per_host(0)
                .build()
                .context("failed to construct dedicated HTTP client")?;
            Ok(client)
        } else {
            Ok(self.shared.clone())
        }
    }

    pub(crate) fn client_for_pool_upstream(&self) -> Client {
        self.pool_upstream.clone()
    }

    pub(crate) fn client_for_forward_proxy(&self, endpoint_url: Option<&Url>) -> Result<Client> {
        let Some(endpoint_url) = endpoint_url else {
            return Ok(self.proxy.clone());
        };

        Self::builder(None, &self.user_agent)
            .pool_max_idle_per_host(2)
            .connect_timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .proxy(
                Proxy::all(endpoint_url.as_str())
                    .with_context(|| format!("invalid forward proxy endpoint: {endpoint_url}"))?,
            )
            .build()
            .context("failed to construct forward proxy HTTP client")
    }

    pub(crate) fn builder(timeout: Option<Duration>, user_agent: &str) -> ClientBuilder {
        let builder = Client::builder()
            .user_agent(user_agent)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(90))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(30))
            .http2_keep_alive_while_idle(true);

        if let Some(timeout) = timeout {
            builder.timeout(timeout)
        } else {
            builder
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_projection_slices_keep_independent_non_extending_deadlines() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        let started = Instant::now();

        hub.mark_dashboard_dirty_at("test", started);
        hub.mark_dashboard_network_dirty_at(started);
        hub.mark_dashboard_terminal_dirty_at(started);
        hub.mark_dashboard_dirty_at("test", started + Duration::from_millis(100));
        hub.mark_dashboard_network_dirty_at(started + Duration::from_millis(500));
        hub.mark_dashboard_terminal_dirty_at(started + Duration::from_secs(2));

        let current = hub
            .pending_dashboard_publish_window()
            .expect("current slice deadline");
        assert_eq!(current.slice, DashboardProjectionSlice::Current);
        assert_eq!(
            current.deadline,
            started + DASHBOARD_RUNTIME_PROJECTION_COALESCE
        );
        hub.complete_dashboard_publish_window(
            hub.begin_dashboard_publish_window(current)
                .expect("consume current window"),
        );

        let network = hub
            .pending_dashboard_publish_window()
            .expect("network slice deadline");
        assert_eq!(network.slice, DashboardProjectionSlice::Network);
        assert_eq!(
            network.deadline,
            started + DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE
        );
        hub.complete_dashboard_publish_window(
            hub.begin_dashboard_publish_window(network)
                .expect("consume network window"),
        );

        let terminal = hub
            .pending_dashboard_publish_window()
            .expect("terminal slice deadline");
        assert_eq!(terminal.slice, DashboardProjectionSlice::Terminal);
        assert_eq!(
            terminal.deadline,
            started + DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE
        );
        hub.complete_dashboard_publish_window(
            hub.begin_dashboard_publish_window(terminal)
                .expect("consume terminal window"),
        );

        assert!(hub.capture_terminal_slice().is_none());
        assert!(hub.capture_terminal_slice().is_none());
        let counters = hub.dashboard_topology_counters();
        assert_eq!(counters.terminal.build_count, 2);
        assert_eq!(counters.terminal.revision_count, 0);
    }

    #[test]
    fn current_slice_comparison_ignores_network_only_changes() {
        let account = DashboardActivityLiveAccount {
            account_key: "upstream:7".to_string(),
            upstream_account_id: Some(7),
            upstream_account_name: Some("account-7".to_string()),
            in_progress_invocation_count: 1,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            upload_bytes_per_second: 1.0,
            download_bytes_per_second: 2.0,
            network_live_bucket: None,
        };
        let mut current = empty_dashboard_live_core();
        current.accounts.push(account.clone());
        let mut network_only = current.clone();
        network_only.accounts[0].upload_bytes_per_second = 99.0;
        network_only.accounts[0].download_bytes_per_second = 101.0;

        assert!(dashboard_current_snapshot_content_eq(
            &current,
            &network_only
        ));

        network_only.accounts[0].in_progress_invocation_count = 2;
        assert!(!dashboard_current_snapshot_content_eq(
            &current,
            &network_only
        ));
    }

    #[test]
    fn current_and_network_slices_use_independent_revisions() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        let network_cache = Arc::new(DashboardNetworkSpeedCache::new(Utc::now()));
        hub.bind_dashboard_network_speed_cache(network_cache.clone())
            .expect("bind network cache");
        {
            let mut dashboard = hub.dashboard.lock().expect("dashboard state");
            let mut core = empty_dashboard_live_core();
            core.in_progress_invocation_count = 1;
            dashboard.live_core = Some(core);
        }

        let current = hub.capture_memory_snapshot().expect("current slice");
        network_cache.record_request_bytes(
            "independent-revision",
            "2026-08-06 10:00:00",
            None,
            Some("api.openai.com"),
            128,
            Utc::now(),
        );
        let network = hub.capture_network_slice().expect("network slice");

        assert_eq!(current.snapshot.revision, 1);
        assert_eq!(network.slice.revision, 1);
    }
}
