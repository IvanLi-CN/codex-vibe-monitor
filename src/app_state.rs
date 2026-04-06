#[derive(Debug)]
struct AppState {
    config: AppConfig,
    pool: Pool<Sqlite>,
    hourly_rollup_sync_lock: Arc<Mutex<()>>,
    http_clients: HttpClients,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    proxy_summary_quota_broadcast_handle: Arc<Mutex<Vec<JoinHandle<()>>>>,
    startup_ready: Arc<AtomicBool>,
    shutdown: CancellationToken,
    semaphore: Arc<Semaphore>,
    proxy_model_settings: Arc<RwLock<ProxyModelSettings>>,
    proxy_model_settings_update_lock: Arc<Mutex<()>>,
    forward_proxy: Arc<Mutex<ForwardProxyManager>>,
    xray_supervisor: Arc<Mutex<XraySupervisor>>,
    forward_proxy_settings_update_lock: Arc<Mutex<()>>,
    forward_proxy_subscription_refresh_lock: Arc<Mutex<()>>,
    pricing_settings_update_lock: Arc<Mutex<()>>,
    pricing_catalog: Arc<RwLock<PricingCatalog>>,
    prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    maintenance_stats_cache: Arc<Mutex<StatsMaintenanceCacheState>>,
    pool_routing_reservations: Arc<std::sync::Mutex<HashMap<String, PoolRoutingReservation>>>,
    pool_group_429_retry_delay_override: Option<Duration>,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
    upstream_accounts: Arc<UpstreamAccountsRuntime>,
}

#[derive(Debug, Clone)]
struct PricingCatalog {
    version: String,
    models: HashMap<String, ModelPricing>,
}

impl Default for PricingCatalog {
    fn default() -> Self {
        Self {
            version: "unavailable".to_string(),
            models: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelPricing {
    input_per_1m: f64,
    output_per_1m: f64,
    #[serde(default)]
    cache_input_per_1m: Option<f64>,
    #[serde(default)]
    reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingEntry {
    model: String,
    input_per_1m: f64,
    output_per_1m: f64,
    #[serde(default)]
    cache_input_per_1m: Option<f64>,
    #[serde(default)]
    reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingSettingsUpdateRequest {
    catalog_version: String,
    #[serde(default)]
    entries: Vec<PricingEntry>,
}

impl PricingSettingsUpdateRequest {
    fn normalized(self) -> Result<PricingCatalog, (StatusCode, String)> {
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
            if let Some(reasoning) = entry.reasoning_per_1m
                && (!reasoning.is_finite() || reasoning < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid reasoningPer1m for model: {model_id}"),
                ));
            }

            let inserted = models.insert(
                model_id.to_string(),
                ModelPricing {
                    input_per_1m: entry.input_per_1m,
                    output_per_1m: entry.output_per_1m,
                    cache_input_per_1m: entry.cache_input_per_1m,
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
struct PricingSettingsResponse {
    catalog_version: String,
    entries: Vec<PricingEntry>,
}

impl PricingSettingsResponse {
    fn from_catalog(catalog: &PricingCatalog) -> Self {
        let mut entries = catalog
            .models
            .iter()
            .map(|(model, pricing)| PricingEntry {
                model: model.clone(),
                input_per_1m: pricing.input_per_1m,
                output_per_1m: pricing.output_per_1m,
                cache_input_per_1m: pricing.cache_input_per_1m,
                reasoning_per_1m: pricing.reasoning_per_1m,
                source: pricing.source.clone(),
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
struct ProxyModelSettings {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    upstream_429_max_retries: u8,
    enabled_preset_models: Vec<String>,
}

fn normalize_proxy_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

fn decode_proxy_upstream_429_max_retries(raw: Option<i64>) -> u8 {
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
            enabled_preset_models: default_enabled_preset_models(),
        }
    }
}

impl ProxyModelSettings {
    fn normalized(self) -> Self {
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
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
struct ProxyModelSettingsRow {
    hijack_enabled: i64,
    merge_upstream_enabled: i64,
    upstream_429_max_retries: Option<i64>,
    enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
            upstream_429_max_retries: decode_proxy_upstream_429_max_retries(
                value.upstream_429_max_retries,
            ),
            enabled_preset_models: decode_enabled_preset_models(
                value.enabled_preset_models_json.as_deref(),
            ),
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsUpdateRequest {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    #[serde(default)]
    upstream_429_max_retries: Option<u8>,
    #[serde(default = "default_enabled_preset_models")]
    enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsResponse {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    upstream_429_max_retries: u8,
    default_hijack_enabled: bool,
    models: Vec<String>,
    enabled_models: Vec<String>,
}

impl From<ProxyModelSettings> for ProxyModelSettingsResponse {
    fn from(value: ProxyModelSettings) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled,
            merge_upstream_enabled: value.merge_upstream_enabled,
            upstream_429_max_retries: value.upstream_429_max_retries,
            default_hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            models: PROXY_PRESET_MODEL_IDS
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            enabled_models: value.enabled_preset_models,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResponse {
    forward_proxy: ForwardProxySettingsResponse,
    pricing: PricingSettingsResponse,
}

fn default_enabled_preset_models() -> Vec<String> {
    PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

fn normalize_enabled_preset_models(enabled_models: Vec<String>) -> Vec<String> {
    let enabled_set: HashSet<&str> = enabled_models.iter().map(String::as_str).collect();
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| enabled_set.contains(**model))
        .map(|model| (*model).to_string())
        .collect()
}

fn decode_enabled_preset_models(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized)
            .map(normalize_enabled_preset_models)
            .unwrap_or_else(|_| default_enabled_preset_models()),
        None => default_enabled_preset_models(),
    }
}


fn default_pricing_source_custom() -> String {
    "custom".to_string()
}

fn normalize_pricing_catalog_version(raw: String) -> Option<String> {
    let normalized = raw.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn normalize_pricing_source(raw: String) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_pricing_source_custom()
    } else {
        normalized
    }
}

#[derive(Debug, Clone)]
struct HttpClients {
    shared: Client,
    pool_upstream: Client,
    proxy: Client,
    timeout: Duration,
    user_agent: String,
}

impl HttpClients {
    fn build(config: &AppConfig) -> Result<Self> {
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

    fn client_for_parallelism(&self, force_new_connection: bool) -> Result<Client> {
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

    fn client_for_pool_upstream(&self) -> Client {
        self.pool_upstream.clone()
    }

    fn client_for_forward_proxy(&self, endpoint_url: Option<&Url>) -> Result<Client> {
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

    fn builder(timeout: Option<Duration>, user_agent: &str) -> ClientBuilder {
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
