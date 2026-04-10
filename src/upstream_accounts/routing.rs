pub(crate) fn pool_routing_timeouts_from_config(
    config: &AppConfig,
) -> PoolRoutingTimeoutSettingsResolved {
    PoolRoutingTimeoutSettingsResolved {
        default_first_byte_timeout: config.request_timeout,
        default_send_timeout: config.openai_proxy_handshake_timeout,
        request_read_timeout: config.openai_proxy_request_read_timeout,
        responses_first_byte_timeout: config.pool_upstream_responses_attempt_timeout,
        compact_first_byte_timeout: config.openai_proxy_compact_handshake_timeout,
        responses_stream_timeout: config.pool_upstream_responses_total_timeout,
        compact_stream_timeout: config.pool_upstream_responses_total_timeout,
    }
}

fn normalize_pool_routing_timeout_secs(
    value: Option<u64>,
    field_name: &str,
) -> Result<Option<u64>, (StatusCode, String)> {
    match value {
        None => Ok(None),
        Some(0) => Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be greater than zero"),
        )),
        Some(value) if value > i64::MAX as u64 => Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be less than or equal to {}", i64::MAX),
        )),
        Some(value) => Ok(Some(value)),
    }
}

fn resolve_pool_routing_timeouts_from_row(
    row: &PoolRoutingSettingsRow,
    config: &AppConfig,
) -> PoolRoutingTimeoutSettingsResolved {
    let defaults = pool_routing_timeouts_from_config(config);
    PoolRoutingTimeoutSettingsResolved {
        responses_first_byte_timeout: row
            .responses_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.responses_first_byte_timeout),
        compact_first_byte_timeout: row
            .compact_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.compact_first_byte_timeout),
        responses_stream_timeout: row
            .responses_stream_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.responses_stream_timeout),
        compact_stream_timeout: row
            .compact_stream_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.compact_stream_timeout),
        default_first_byte_timeout: row
            .default_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.default_first_byte_timeout),
        default_send_timeout: row
            .upstream_handshake_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.default_send_timeout),
        request_read_timeout: row
            .request_read_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.request_read_timeout),
    }
}

fn pool_routing_timeouts_response(
    resolved: PoolRoutingTimeoutSettingsResolved,
) -> PoolRoutingTimeoutSettingsResponse {
    PoolRoutingTimeoutSettingsResponse {
        responses_first_byte_timeout_secs: resolved.responses_first_byte_timeout.as_secs(),
        compact_first_byte_timeout_secs: resolved.compact_first_byte_timeout.as_secs(),
        responses_stream_timeout_secs: resolved.responses_stream_timeout.as_secs(),
        compact_stream_timeout_secs: resolved.compact_stream_timeout.as_secs(),
    }
}

async fn load_pool_routing_settings(pool: &Pool<Sqlite>) -> Result<PoolRoutingSettingsRow> {
    sqlx::query_as::<_, PoolRoutingSettingsRow>(
        r#"
        SELECT
            encrypted_api_key,
            masked_api_key,
            primary_sync_interval_secs,
            secondary_sync_interval_secs,
            priority_available_account_cap,
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            default_first_byte_timeout_secs,
            upstream_handshake_timeout_secs,
            request_read_timeout_secs
        FROM pool_routing_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

fn resolve_pool_routing_maintenance_settings(
    row: &PoolRoutingSettingsRow,
    config: &AppConfig,
) -> PoolRoutingMaintenanceSettings {
    let primary_sync_interval_secs = row
        .primary_sync_interval_secs
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(config.upstream_accounts_sync_interval.as_secs())
        .max(MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS);
    let secondary_default =
        DEFAULT_UPSTREAM_ACCOUNTS_SECONDARY_SYNC_INTERVAL_SECS.max(primary_sync_interval_secs);
    let secondary_sync_interval_secs = row
        .secondary_sync_interval_secs
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(secondary_default)
        .max(primary_sync_interval_secs)
        .max(MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS);
    let priority_available_account_cap = row
        .priority_available_account_cap
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_UPSTREAM_ACCOUNTS_PRIORITY_AVAILABLE_ACCOUNT_CAP);

    PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs,
        secondary_sync_interval_secs,
        priority_available_account_cap,
    }
}

fn build_pool_routing_settings_response(
    state: &AppState,
    row: &PoolRoutingSettingsRow,
) -> PoolRoutingSettingsResponse {
    let timeouts = resolve_pool_routing_timeouts_from_row(row, &state.config);
    PoolRoutingSettingsResponse {
        writes_enabled: true,
        api_key_configured: row
            .encrypted_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        masked_api_key: row.masked_api_key.clone(),
        maintenance: resolve_pool_routing_maintenance_settings(row, &state.config).into_response(),
        timeouts: pool_routing_timeouts_response(timeouts),
    }
}

fn validate_pool_routing_maintenance_settings(
    settings: PoolRoutingMaintenanceSettings,
) -> Result<(), (StatusCode, String)> {
    if settings.primary_sync_interval_secs < MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "maintenance.primarySyncIntervalSecs must be >= {MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS}"
            ),
        ));
    }
    if settings.secondary_sync_interval_secs < MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "maintenance.secondarySyncIntervalSecs must be >= {MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS}"
            ),
        ));
    }
    if settings.secondary_sync_interval_secs < settings.primary_sync_interval_secs {
        return Err((
            StatusCode::BAD_REQUEST,
            "maintenance.secondarySyncIntervalSecs must be >= maintenance.primarySyncIntervalSecs"
                .to_string(),
        ));
    }
    if settings.priority_available_account_cap == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "maintenance.priorityAvailableAccountCap must be >= 1".to_string(),
        ));
    }
    Ok(())
}

fn merge_pool_routing_maintenance_settings(
    current: PoolRoutingMaintenanceSettings,
    patch: Option<&UpdatePoolRoutingMaintenanceSettingsRequest>,
) -> PoolRoutingMaintenanceSettings {
    let Some(patch) = patch else {
        return current;
    };
    PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: patch
            .primary_sync_interval_secs
            .unwrap_or(current.primary_sync_interval_secs),
        secondary_sync_interval_secs: patch
            .secondary_sync_interval_secs
            .unwrap_or(current.secondary_sync_interval_secs),
        priority_available_account_cap: patch
            .priority_available_account_cap
            .unwrap_or(current.priority_available_account_cap),
    }
}

async fn load_pool_routing_settings_seeded(
    pool: &Pool<Sqlite>,
    _config: &AppConfig,
) -> Result<PoolRoutingSettingsRow> {
    load_pool_routing_settings(pool).await
}

pub(crate) async fn resolve_pool_routing_timeouts(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<PoolRoutingTimeoutSettingsResolved> {
    let row = load_pool_routing_settings_seeded(pool, config).await?;
    Ok(resolve_pool_routing_timeouts_from_row(&row, config))
}

fn build_pool_routing_runtime_cache(
    state: &AppState,
    row: &PoolRoutingSettingsRow,
) -> Result<PoolRoutingRuntimeCache> {
    let api_key = match (
        state.upstream_accounts.crypto_key.as_ref(),
        row.encrypted_api_key.as_deref(),
    ) {
        (Some(crypto_key), Some(encrypted_api_key)) => {
            Some(decrypt_secret_value(crypto_key, encrypted_api_key)?)
        }
        _ => None,
    };

    Ok(PoolRoutingRuntimeCache {
        api_key,
        timeouts: resolve_pool_routing_timeouts_from_row(row, &state.config),
    })
}

pub(crate) async fn refresh_pool_routing_runtime_cache(
    state: &AppState,
) -> Result<PoolRoutingRuntimeCache> {
    let row = match load_pool_routing_settings_seeded(&state.pool, &state.config).await {
        Ok(row) => row,
        Err(err) => {
            let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
            *runtime_cache = None;
            return Err(err);
        }
    };
    let cache = match build_pool_routing_runtime_cache(state, &row) {
        Ok(cache) => cache,
        Err(err) => {
            let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
            *runtime_cache = None;
            return Err(err);
        }
    };
    let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
    *runtime_cache = Some(cache.clone());
    Ok(cache)
}

pub(crate) async fn load_pool_routing_runtime_cache(
    state: &AppState,
) -> Result<PoolRoutingRuntimeCache> {
    {
        let runtime_cache = state.pool_routing_runtime_cache.lock().await;
        if let Some(cache) = runtime_cache.as_ref() {
            return Ok(cache.clone());
        }
    }

    refresh_pool_routing_runtime_cache(state).await
}

async fn save_pool_routing_settings(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    crypto_key: Option<&[u8; 32]>,
    api_key: Option<&str>,
    timeout_updates: Option<&UpdatePoolRoutingTimeoutSettingsRequest>,
) -> Result<PoolRoutingSettingsRow, (StatusCode, String)> {
    let current = load_pool_routing_settings_seeded(pool, config)
        .await
        .map_err(internal_error_tuple)?;
    let encrypted_api_key = match api_key {
        Some(api_key) => {
            let crypto_key = crypto_key.ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "pool routing secret storage is unavailable".to_string(),
                )
            })?;
            Some(encrypt_secret_value(crypto_key, api_key).map_err(internal_error_tuple)?)
        }
        None => current.encrypted_api_key.clone(),
    };
    let masked_api_key = match api_key {
        Some(api_key) => Some(mask_api_key(api_key)),
        None => current.masked_api_key.clone(),
    };
    let primary_sync_interval_secs = current.primary_sync_interval_secs;
    let secondary_sync_interval_secs = current.secondary_sync_interval_secs;
    let priority_available_account_cap = current.priority_available_account_cap;
    let responses_first_byte_timeout_secs = timeout_updates
        .and_then(|value| value.responses_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.responses_first_byte_timeout_secs);
    let compact_first_byte_timeout_secs = timeout_updates
        .and_then(|value| value.compact_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.compact_first_byte_timeout_secs);
    let responses_stream_timeout_secs = timeout_updates
        .and_then(|value| value.responses_stream_timeout_secs)
        .map(|value| value as i64)
        .or(current.responses_stream_timeout_secs);
    let compact_stream_timeout_secs = timeout_updates
        .and_then(|value| value.compact_stream_timeout_secs)
        .map(|value| value as i64)
        .or(current.compact_stream_timeout_secs);
    let default_first_byte_timeout_secs = current.default_first_byte_timeout_secs;
    let upstream_handshake_timeout_secs = current.upstream_handshake_timeout_secs;
    let request_read_timeout_secs = current.request_read_timeout_secs;
    let now_iso = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET encrypted_api_key = ?2,
            masked_api_key = ?3,
            primary_sync_interval_secs = ?4,
            secondary_sync_interval_secs = ?5,
            priority_available_account_cap = ?6,
            responses_first_byte_timeout_secs = ?7,
            compact_first_byte_timeout_secs = ?8,
            responses_stream_timeout_secs = ?9,
            compact_stream_timeout_secs = ?10,
            default_first_byte_timeout_secs = ?11,
            upstream_handshake_timeout_secs = ?12,
            request_read_timeout_secs = ?13,
            updated_at = ?14
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind(encrypted_api_key)
    .bind(masked_api_key)
    .bind(primary_sync_interval_secs)
    .bind(secondary_sync_interval_secs)
    .bind(priority_available_account_cap)
    .bind(responses_first_byte_timeout_secs)
    .bind(compact_first_byte_timeout_secs)
    .bind(responses_stream_timeout_secs)
    .bind(compact_stream_timeout_secs)
    .bind(default_first_byte_timeout_secs)
    .bind(upstream_handshake_timeout_secs)
    .bind(request_read_timeout_secs)
    .bind(now_iso)
    .execute(pool)
    .await
    .map_err(internal_error_tuple)?;

    load_pool_routing_settings(pool)
        .await
        .map_err(internal_error_tuple)
}

async fn save_pool_routing_api_key(
    pool: &Pool<Sqlite>,
    crypto_key: &[u8; 32],
    api_key: &str,
) -> Result<()> {
    let encrypted_api_key = encrypt_secret_value(crypto_key, api_key)?;
    let masked_api_key = mask_api_key(api_key);
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET encrypted_api_key = ?2,
            masked_api_key = ?3,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind(encrypted_api_key)
    .bind(masked_api_key)
    .bind(now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn save_pool_routing_maintenance_settings(
    pool: &Pool<Sqlite>,
    settings: PoolRoutingMaintenanceSettings,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET primary_sync_interval_secs = ?2,
            secondary_sync_interval_secs = ?3,
            priority_available_account_cap = ?4,
            updated_at = ?5
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind(i64::try_from(settings.primary_sync_interval_secs)?)
    .bind(i64::try_from(settings.secondary_sync_interval_secs)?)
    .bind(i64::try_from(settings.priority_available_account_cap)?)
    .bind(now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn pool_api_key_matches(state: &AppState, api_key: &str) -> Result<bool> {
    let runtime_cache = load_pool_routing_runtime_cache(state).await?;
    let Some(expected_api_key) = runtime_cache.api_key.as_deref() else {
        return Ok(false);
    };
    Ok(expected_api_key == api_key.trim())
}

#[derive(Debug, Clone)]
pub(crate) enum PoolResolvedAuth {
    ApiKey {
        authorization: String,
    },
    Oauth {
        access_token: String,
        chatgpt_account_id: Option<String>,
    },
}

impl PoolResolvedAuth {
    pub(crate) fn authorization_header_value(&self) -> Option<&str> {
        match self {
            Self::ApiKey { authorization } => Some(authorization.as_str()),
            Self::Oauth { .. } => None,
        }
    }

    pub(crate) fn is_oauth(&self) -> bool {
        matches!(self, Self::Oauth { .. })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PoolResolvedAccount {
    pub(crate) account_id: i64,
    pub(crate) display_name: String,
    pub(crate) kind: String,
    pub(crate) auth: PoolResolvedAuth,
    pub(crate) group_name: Option<String>,
    pub(crate) bound_proxy_keys: Vec<String>,
    pub(crate) forward_proxy_scope: ForwardProxyRouteScope,
    pub(crate) group_upstream_429_retry_enabled: bool,
    pub(crate) group_upstream_429_max_retries: u8,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) upstream_base_url: Url,
    pub(crate) routing_source: PoolRoutingSelectionSource,
}

impl PoolResolvedAccount {
    pub(crate) fn upstream_route_key(&self) -> String {
        canonical_pool_upstream_route_key(&self.upstream_base_url)
    }

    pub(crate) fn effective_group_upstream_429_max_retries(&self) -> u8 {
        normalize_group_upstream_429_retry_metadata(
            self.group_upstream_429_retry_enabled,
            self.group_upstream_429_max_retries,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingSelectionSource {
    StickyReuse,
    FreshAssignment,
}

#[derive(Debug, Clone)]
pub(crate) enum PoolAccountResolution {
    Resolved(PoolResolvedAccount),
    RateLimited,
    DegradedOnly,
    Unavailable,
    NoCandidate,
    BlockedByPolicy(String),
}

#[derive(Debug, Clone)]
enum PoolAccountGroupProxyRoutingReadiness {
    Ready(UpstreamAccountGroupMetadata),
    Blocked(String),
}

async fn load_account_group_name_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, Option<String>>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, group_name FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(")")
        .build_query_as::<(i64, Option<String>)>()
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

async fn load_effective_routing_rules_for_accounts(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, EffectiveRoutingRule>> {
    let account_group_map = load_account_group_name_map(pool, account_ids).await?;
    if account_group_map.is_empty() {
        return Ok(HashMap::new());
    }

    let tags_by_account = load_account_tag_map(pool, account_ids).await?;
    let group_names = account_group_map
        .values()
        .filter_map(|group_name| normalize_optional_text(group_name.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let group_metadata = load_group_metadata_map(pool, &group_names).await?;
    let mut rules = HashMap::with_capacity(account_group_map.len());
    for (account_id, group_name) in account_group_map {
        let mut rule = build_effective_routing_rule(
            tags_by_account
                .get(&account_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
        let group_concurrency_limit = normalize_optional_text(group_name)
            .and_then(|name| group_metadata.get(&name))
            .map(|metadata| metadata.concurrency_limit)
            .unwrap_or_default();
        rule.concurrency_limit =
            merge_concurrency_limits(rule.concurrency_limit, group_concurrency_limit);
        rules.insert(account_id, rule);
    }
    Ok(rules)
}

fn routing_priority_rank(rule: Option<&EffectiveRoutingRule>) -> u8 {
    rule.map(|rule| rule.priority_tier)
        .unwrap_or_default()
        .routing_rank()
}

async fn load_effective_routing_rule_for_account(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<EffectiveRoutingRule> {
    Ok(
        load_effective_routing_rules_for_accounts(pool, &[account_id])
            .await?
            .remove(&account_id)
            .unwrap_or_else(|| build_effective_routing_rule(&[])),
    )
}

fn account_accepts_concurrency_limit(
    effective_load: i64,
    routing_source: PoolRoutingSelectionSource,
    rule: &EffectiveRoutingRule,
) -> bool {
    routing_source == PoolRoutingSelectionSource::StickyReuse
        || rule.concurrency_limit == 0
        || effective_load < rule.concurrency_limit
}

async fn account_accepts_sticky_assignment(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    source_account_id: Option<i64>,
    rule: &EffectiveRoutingRule,
) -> Result<bool> {
    let Some(_) = sticky_key else {
        return Ok(true);
    };
    let is_transfer = source_account_id.is_some_and(|source_id| source_id != account_id);
    let is_new_assignment = source_account_id.is_none();
    if !is_transfer && !is_new_assignment {
        return Ok(true);
    }
    if is_transfer && !rule.allow_cut_in {
        return Ok(false);
    }
    for guard in &rule.guard_rules {
        let current =
            count_recent_account_conversations(pool, account_id, guard.lookback_hours).await?;
        if current >= guard.max_conversations {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn resolve_pool_account_group_proxy_routing_readiness(
    state: &AppState,
    group_name: Option<&str>,
) -> Result<PoolAccountGroupProxyRoutingReadiness> {
    let normalized_group_name = group_name.map(str::trim).filter(|value| !value.is_empty());
    let group_metadata = load_group_metadata(&state.pool, normalized_group_name).await?;
    if group_metadata.node_shunt_enabled {
        if normalized_group_name.is_none() {
            return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
                missing_account_group_error_message(),
            ));
        }
        return Ok(PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata));
    }
    let Some(group_name) = normalized_group_name else {
        return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
            missing_account_group_error_message(),
        ));
    };
    let scope = match load_required_account_forward_proxy_scope_from_group_metadata(
        state,
        Some(group_name),
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
                err.to_string(),
            ));
        }
    };
    let ForwardProxyRouteScope::BoundGroup {
        group_name,
        bound_proxy_keys,
    } = &scope
    else {
        unreachable!("strict pool account routing should never fall back to automatic");
    };
    let has_selectable_bound_proxy_keys = {
        let manager = state.forward_proxy.lock().await;
        manager.has_selectable_bound_proxy_keys(bound_proxy_keys)
    };
    if !has_selectable_bound_proxy_keys {
        return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
            missing_selectable_group_bound_proxy_error_message(group_name),
        ));
    }
    Ok(PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata))
}

fn summarize_pool_group_proxy_blocked_messages(messages: &[String]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut unique_messages = Vec::new();
    for message in messages {
        let normalized = message.trim();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_string()) {
            unique_messages.push(normalized.to_string());
        }
    }
    let first_message = unique_messages.first()?.clone();
    if unique_messages.len() == 1 {
        return Some(first_message);
    }
    Some(format!(
        "{first_message}; plus {} additional upstream account group routing configuration issue(s)",
        unique_messages.len() - 1
    ))
}

pub(crate) async fn resolve_pool_account_for_request(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement(
        state,
        sticky_key,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
) -> Result<PoolAccountResolution> {
    let now = Utc::now();
    let mut tried = excluded_ids.iter().copied().collect::<HashSet<_>>();
    let mut saw_rate_limited_candidate = false;
    let mut saw_degraded_candidate = false;
    let mut saw_other_non_rate_limited_routing_candidate = false;
    let mut saw_excluded_route_candidate = false;
    let mut saw_non_required_route_candidate = false;
    let mut saw_non_routing_candidate = false;
    let mut sticky_route_excluded_by_route_key = false;
    let mut sticky_route_still_reusable = false;
    let mut sticky_route_group_proxy_blocked_message = None;
    let mut group_proxy_blocked_messages = Vec::new();
    let mut node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;

    let sticky_route = if let Some(sticky_key) = sticky_key {
        load_sticky_route(&state.pool, sticky_key).await?
    } else {
        None
    };
    let sticky_source_id = sticky_route.as_ref().map(|route| route.account_id);
    let sticky_source_rule = if let Some(route) = sticky_route.as_ref() {
        Some(load_effective_routing_rule_for_account(&state.pool, route.account_id).await?)
    } else {
        None
    };

    if let Some(route) = sticky_route.as_ref() {
        if !tried.contains(&route.account_id)
            && let Some(row) = load_upstream_account_row(&state.pool, route.account_id).await?
        {
            tried.insert(route.account_id);
            let sticky_candidate =
                load_account_routing_candidate(&state.pool, route.account_id).await?;
            let sticky_snapshot_exhausted = sticky_candidate
                .as_ref()
                .is_some_and(routing_candidate_snapshot_is_exhausted);
            let sticky_route_key = resolve_pool_account_upstream_base_url(
                &row,
                &state.config.openai_upstream_base_url,
            )
            .ok()
            .map(|url| canonical_pool_upstream_route_key(&url));
            let sticky_route_matches_required =
                required_upstream_route_key.is_none_or(|required| {
                    sticky_route_key
                        .as_deref()
                        .is_some_and(|route_key| route_key == required)
                });
            let sticky_route_is_excluded_by_route_key = sticky_route_key
                .as_deref()
                .is_some_and(|route_key| excluded_upstream_route_keys.contains(route_key));
            if !sticky_route_matches_required {
                if is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted)
                    || is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                    || is_routing_eligible_account(&row)
                {
                    saw_non_required_route_candidate = true;
                } else if is_pool_account_routing_candidate(&row) {
                    saw_non_routing_candidate = true;
                }
            } else if is_account_selectable_for_sticky_reuse(&row, sticky_snapshot_exhausted, now) {
                sticky_route_still_reusable = true;
                let mut sticky_route_was_excluded = false;
                match resolve_pool_account_group_proxy_routing_readiness(
                    state,
                    row.group_name.as_deref(),
                )
                .await?
                {
                    PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => {
                        let prepared_account = prepare_pool_account_with_node_shunt_refresh(
                            state,
                            &row,
                            sticky_source_rule
                                .as_ref()
                                .expect("sticky source rule should be loaded"),
                            &group_metadata,
                            &mut node_shunt_assignments,
                        )
                        .await;
                        let account = match prepared_account {
                            Ok(account) => account,
                            Err(err)
                                if is_group_node_shunt_unassigned_message(&err.to_string()) =>
                            {
                                sticky_route_group_proxy_blocked_message = Some(err.to_string());
                                group_proxy_blocked_messages.push(err.to_string());
                                None
                            }
                            Err(err) => return Err(err),
                        };
                        if let Some(account) = account {
                            let mut account = account;
                            account.routing_source = PoolRoutingSelectionSource::StickyReuse;
                            if !excluded_upstream_route_keys.contains(&account.upstream_route_key())
                            {
                                return Ok(PoolAccountResolution::Resolved(account));
                            }
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                            if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                            {
                                saw_degraded_candidate = true;
                            } else {
                                saw_excluded_route_candidate = true;
                            }
                        }
                    }
                    PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                        if sticky_route_is_excluded_by_route_key {
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                            saw_excluded_route_candidate = true;
                        } else {
                            sticky_route_group_proxy_blocked_message = Some(message.clone());
                            group_proxy_blocked_messages.push(message);
                        }
                    }
                }
                if !sticky_route_was_excluded {
                    if sticky_route_group_proxy_blocked_message.is_none() {
                        if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                            saw_degraded_candidate = true;
                        } else {
                            saw_other_non_rate_limited_routing_candidate = true;
                        }
                    }
                }
            } else if sticky_route_is_excluded_by_route_key
                && (is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted)
                    || is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                    || is_routing_eligible_account(&row))
            {
                saw_excluded_route_candidate = true;
            } else if is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted) {
                saw_rate_limited_candidate = true;
            } else if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                saw_degraded_candidate = true;
            } else if is_routing_eligible_account(&row) {
                saw_other_non_rate_limited_routing_candidate = true;
            } else if is_pool_account_routing_candidate(&row) {
                // Active accounts without usable credentials are not real
                // routing candidates and should not mask an all-429 pool.
                saw_non_routing_candidate = true;
            }
        }
        if sticky_source_rule
            .as_ref()
            .is_some_and(|rule| !rule.allow_cut_out)
            && sticky_route_still_reusable
            && !sticky_route_excluded_by_route_key
        {
            if let Some(message) = sticky_route_group_proxy_blocked_message {
                return Ok(PoolAccountResolution::BlockedByPolicy(message));
            }
            return Ok(PoolAccountResolution::BlockedByPolicy(
                "sticky conversation cannot cut out of the current account because a tag rule forbids it"
                    .to_string(),
            ));
        }
    }

    let mut candidates = load_account_routing_candidates(&state.pool, &tried).await?;
    for candidate in &mut candidates {
        candidate.in_flight_reservations = pool_routing_reservation_count(state, candidate.id);
    }
    let candidate_effective_rules = load_effective_routing_rules_for_accounts(
        &state.pool,
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
    )
    .await?;
    candidates.sort_by(compare_routing_candidates);
    let mut primary_candidates = [Vec::new(), Vec::new(), Vec::new()];
    let mut overflow_candidates = [Vec::new(), Vec::new(), Vec::new()];
    for candidate in candidates {
        let priority_index = usize::from(routing_priority_rank(
            candidate_effective_rules.get(&candidate.id),
        ));
        if candidate.effective_load() < candidate.capacity_profile().hard_cap {
            primary_candidates[priority_index].push(candidate);
        } else {
            overflow_candidates[priority_index].push(candidate);
        }
    }
    let mut candidate_passes = Vec::new();
    for priority_index in 0..=2 {
        if primary_candidates[priority_index].is_empty() {
            if !overflow_candidates[priority_index].is_empty() {
                candidate_passes.push(std::mem::take(&mut overflow_candidates[priority_index]));
            }
            continue;
        }
        candidate_passes.push(std::mem::take(&mut primary_candidates[priority_index]));
        if !overflow_candidates[priority_index].is_empty() {
            candidate_passes.push(std::mem::take(&mut overflow_candidates[priority_index]));
        }
    }
    for pass_candidates in candidate_passes {
        for candidate in pass_candidates {
            let Some(row) = load_upstream_account_row(&state.pool, candidate.id).await? else {
                continue;
            };
            let snapshot_exhausted = routing_candidate_snapshot_is_exhausted(&candidate);
            let candidate_route_key = resolve_pool_account_upstream_base_url(
                &row,
                &state.config.openai_upstream_base_url,
            )
            .ok()
            .map(|url| canonical_pool_upstream_route_key(&url));
            let candidate_route_matches_required =
                required_upstream_route_key.is_none_or(|required| {
                    candidate_route_key
                        .as_deref()
                        .is_some_and(|route_key| route_key == required)
                });
            let candidate_route_is_excluded_by_route_key = candidate_route_key
                .as_deref()
                .is_some_and(|route_key| excluded_upstream_route_keys.contains(route_key));
            if !candidate_route_matches_required {
                if is_account_rate_limited_for_routing(&row, snapshot_exhausted)
                    || is_account_degraded_for_routing(&row, snapshot_exhausted, now)
                    || is_routing_eligible_account(&row)
                {
                    saw_non_required_route_candidate = true;
                } else {
                    saw_non_routing_candidate = true;
                }
                continue;
            }
            if !is_account_selectable_for_fresh_assignment(&row, snapshot_exhausted, now) {
                if candidate_route_is_excluded_by_route_key
                    && (is_account_rate_limited_for_routing(&row, snapshot_exhausted)
                        || is_account_degraded_for_routing(&row, snapshot_exhausted, now)
                        || is_routing_eligible_account(&row))
                {
                    saw_excluded_route_candidate = true;
                } else if is_account_rate_limited_for_routing(&row, snapshot_exhausted) {
                    saw_rate_limited_candidate = true;
                } else if is_account_degraded_for_routing(&row, snapshot_exhausted, now) {
                    saw_degraded_candidate = true;
                } else if is_routing_eligible_account(&row) {
                    saw_other_non_rate_limited_routing_candidate = true;
                } else {
                    saw_non_routing_candidate = true;
                }
                continue;
            }
            let Some(effective_rule) = candidate_effective_rules.get(&row.id) else {
                continue;
            };
            if !account_accepts_concurrency_limit(
                candidate.effective_load(),
                PoolRoutingSelectionSource::FreshAssignment,
                effective_rule,
            ) {
                saw_other_non_rate_limited_routing_candidate = true;
                continue;
            }
            if !account_accepts_sticky_assignment(
                &state.pool,
                row.id,
                sticky_key,
                sticky_source_id,
                effective_rule,
            )
            .await?
            {
                if candidate_route_is_excluded_by_route_key {
                    saw_excluded_route_candidate = true;
                } else {
                    saw_other_non_rate_limited_routing_candidate = true;
                }
                continue;
            }
            let group_metadata = match resolve_pool_account_group_proxy_routing_readiness(
                state,
                row.group_name.as_deref(),
            )
            .await?
            {
                PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => group_metadata,
                PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                    if candidate_route_is_excluded_by_route_key {
                        saw_excluded_route_candidate = true;
                    } else {
                        group_proxy_blocked_messages.push(message);
                    }
                    continue;
                }
            };
            let prepared_account = prepare_pool_account_with_node_shunt_refresh(
                state,
                &row,
                effective_rule,
                &group_metadata,
                &mut node_shunt_assignments,
            )
            .await;
            let account = match prepared_account {
                Ok(account) => account,
                Err(err) if is_group_node_shunt_unassigned_message(&err.to_string()) => {
                    if candidate_route_is_excluded_by_route_key {
                        saw_excluded_route_candidate = true;
                    } else {
                        group_proxy_blocked_messages.push(err.to_string());
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            if let Some(account) = account {
                if excluded_upstream_route_keys.contains(&account.upstream_route_key()) {
                    saw_excluded_route_candidate = true;
                    continue;
                }
                return Ok(PoolAccountResolution::Resolved(account));
            }
            saw_other_non_rate_limited_routing_candidate = true;
        }
    }

    // Surface concrete group-proxy misconfiguration before generic pool exhaustion
    // when every transferable fresh candidate was filtered for that reason,
    // even if the rest of the pool is already rate-limited or degraded.
    if !saw_other_non_rate_limited_routing_candidate
        && let Some(message) =
            summarize_pool_group_proxy_blocked_messages(&group_proxy_blocked_messages)
    {
        return Ok(PoolAccountResolution::BlockedByPolicy(message));
    }
    if saw_rate_limited_candidate
        && !saw_degraded_candidate
        && !saw_other_non_rate_limited_routing_candidate
        && !saw_excluded_route_candidate
    {
        return Ok(PoolAccountResolution::RateLimited);
    }
    if saw_degraded_candidate
        && !saw_rate_limited_candidate
        && !saw_other_non_rate_limited_routing_candidate
        && !saw_excluded_route_candidate
        && !saw_non_routing_candidate
    {
        return Ok(PoolAccountResolution::DegradedOnly);
    }
    if saw_other_non_rate_limited_routing_candidate
        || saw_non_required_route_candidate
        || saw_excluded_route_candidate
        || saw_non_routing_candidate
        || (saw_rate_limited_candidate && saw_degraded_candidate)
    {
        return Ok(PoolAccountResolution::Unavailable);
    }

    Ok(PoolAccountResolution::NoCandidate)
}

pub(crate) async fn record_pool_route_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    request_started_at_utc: DateTime<Utc>,
    sticky_key: Option<&str>,
    invoke_id: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    let request_started_at_iso = format_utc_iso(request_started_at_utc);
    let update_result = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_selected_at = COALESCE(last_selected_at, ?3),
            last_error = NULL,
            last_error_at = NULL,
            last_route_failure_at = NULL,
            last_route_failure_kind = NULL,
            cooldown_until = NULL,
            consecutive_route_failures = 0,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1
          AND (
                last_route_failure_at IS NULL
                OR last_route_failure_at <= ?4
            )
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .bind(&request_started_at_iso)
    .execute(pool)
    .await?;
    if update_result.rows_affected() == 0 {
        return Ok(());
    }
    if let Some(sticky_key) = sticky_key {
        upsert_sticky_route(pool, sticky_key, account_id, &now_iso).await?;
    }
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: None,
            reason_message: None,
            http_status: None,
            failure_kind: None,
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn record_pool_route_http_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    account_kind: &str,
    sticky_key: Option<&str>,
    status: StatusCode,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    if route_http_failure_is_retryable_server_overloaded(status, error_message) {
        return record_pool_route_retryable_overload_failure(
            pool,
            account_id,
            sticky_key,
            error_message,
            invoke_id,
        )
        .await;
    }

    let classification = classify_pool_account_http_failure(account_kind, status, error_message);
    match classification.disposition {
        UpstreamAccountFailureDisposition::HardUnavailable => {
            if let Some(sticky_key) = sticky_key {
                delete_sticky_route(pool, sticky_key).await?;
            }
            let now_iso = format_utc_iso(Utc::now());
            sqlx::query(
                r#"
                UPDATE pool_upstream_accounts
                SET status = ?2,
                    last_error = ?3,
                    last_error_at = ?4,
                    last_route_failure_at = ?4,
                    last_route_failure_kind = ?5,
                    cooldown_until = NULL,
                    consecutive_route_failures = consecutive_route_failures + 1,
                    temporary_route_failure_streak_started_at = NULL,
                    updated_at = ?4
                WHERE id = ?1
                "#,
            )
            .bind(account_id)
            .bind(
                classification
                    .next_account_status
                    .unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR),
            )
            .bind(error_message)
            .bind(&now_iso)
            .bind(classification.failure_kind)
            .execute(pool)
            .await?;
            record_upstream_account_action(
                pool,
                account_id,
                UpstreamAccountActionPayload {
                    action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                    source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                    reason_code: Some(classification.reason_code),
                    reason_message: Some(error_message),
                    http_status: Some(status),
                    failure_kind: Some(classification.failure_kind),
                    invoke_id,
                    sticky_key,
                    occurred_at: &now_iso,
                },
            )
            .await?;
            Ok(())
        }
        UpstreamAccountFailureDisposition::RateLimited
        | UpstreamAccountFailureDisposition::Retryable => {
            let base_secs = if status == StatusCode::TOO_MANY_REQUESTS {
                15
            } else {
                5
            };
            apply_pool_route_cooldown_failure(
                pool,
                account_id,
                sticky_key,
                error_message,
                classification.failure_kind,
                classification.reason_code,
                status,
                base_secs,
                invoke_id,
            )
            .await
        }
    }
}

pub(crate) async fn record_pool_route_retryable_overload_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
        StatusCode::OK,
        5,
        invoke_id,
    )
    .await
}

pub(crate) async fn record_pool_route_transport_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    invoke_id: Option<&str>,
) -> Result<()> {
    apply_pool_route_cooldown_failure(
        pool,
        account_id,
        sticky_key,
        error_message,
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
        StatusCode::BAD_GATEWAY,
        5,
        invoke_id,
    )
    .await
}

pub(crate) async fn build_account_sticky_keys_response(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selection: AccountStickyKeySelection,
) -> Result<AccountStickyKeysResponse> {
    let range_end = Utc::now();
    let range_start = range_end - ChronoDuration::hours(selection.activity_window_hours());
    let range_start_bound = db_occurred_at_lower_bound(range_start);
    let routes = load_account_sticky_routes(pool, account_id).await?;
    if routes.is_empty() {
        return Ok(AccountStickyKeysResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            selection_mode: selection.selection_mode(),
            selected_limit: selection.selected_limit(),
            selected_activity_hours: selection.selected_activity_hours(),
            implicit_filter: selection.implicit_filter(AccountStickyKeyFilteredCounts::default()),
            conversations: Vec::new(),
        });
    }

    let attached_keys = routes
        .iter()
        .map(|row| row.sticky_key.clone())
        .collect::<Vec<_>>();
    let aggregates = query_account_sticky_key_aggregates(pool, account_id, &attached_keys).await?;
    let events =
        query_account_sticky_key_events(pool, account_id, &range_start_bound, &attached_keys)
            .await?;

    let mut aggregate_map = aggregates
        .into_iter()
        .map(|row| (row.sticky_key.clone(), row))
        .collect::<HashMap<_, _>>();
    let mut grouped_events: HashMap<String, Vec<AccountStickyKeyRequestPoint>> = HashMap::new();
    for row in events {
        let status = if row.status.trim().is_empty() {
            "unknown".to_string()
        } else {
            row.status.trim().to_string()
        };
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.sticky_key.clone()).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(AccountStickyKeyRequestPoint {
            occurred_at: row.occurred_at,
            status: status.clone(),
            is_success: status.eq_ignore_ascii_case("success"),
            request_tokens,
            cumulative_tokens,
        });
    }

    let mut conversations = routes
        .into_iter()
        .map(|route| {
            let aggregate = aggregate_map.remove(&route.sticky_key);
            let last24h_requests = grouped_events.remove(&route.sticky_key).unwrap_or_default();
            AccountStickyKeyConversation {
                sticky_key: route.sticky_key.clone(),
                request_count: aggregate.as_ref().map(|row| row.request_count).unwrap_or(0),
                total_tokens: aggregate.as_ref().map(|row| row.total_tokens).unwrap_or(0),
                total_cost: aggregate.as_ref().map(|row| row.total_cost).unwrap_or(0.0),
                created_at: aggregate
                    .as_ref()
                    .map(|row| row.created_at.clone())
                    .unwrap_or_else(|| route.created_at.clone()),
                last_activity_at: aggregate
                    .as_ref()
                    .map(|row| row.last_activity_at.clone())
                    .unwrap_or_else(|| route.last_seen_at.clone()),
                recent_invocations: Vec::new(),
                last24h_requests,
            }
        })
        .collect::<Vec<_>>();
    conversations.sort_by(|left, right| {
        let left_last_24h = left
            .last24h_requests
            .last()
            .map(|point| point.occurred_at.as_str())
            .unwrap_or("");
        let right_last_24h = right
            .last24h_requests
            .last()
            .map(|point| point.occurred_at.as_str())
            .unwrap_or("");
        right_last_24h
            .cmp(left_last_24h)
            .then_with(|| right.last_activity_at.cmp(&left.last_activity_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.sticky_key.cmp(&right.sticky_key))
    });

    let mut filtered_counts = AccountStickyKeyFilteredCounts::default();
    if matches!(selection, AccountStickyKeySelection::ActivityWindow(_)) {
        filtered_counts.inactive_count = conversations
            .iter()
            .filter(|conversation| conversation.last24h_requests.is_empty())
            .count() as i64;
        conversations.retain(|conversation| !conversation.last24h_requests.is_empty());
    }

    filtered_counts.capped_count = conversations
        .len()
        .saturating_sub(selection.display_limit().max(0) as usize)
        as i64;
    conversations.truncate(selection.display_limit().max(0) as usize);

    let selected_keys = conversations
        .iter()
        .map(|conversation| conversation.sticky_key.clone())
        .collect::<Vec<_>>();
    let preview_range_start_bound = match selection {
        AccountStickyKeySelection::ActivityWindow(_) => Some(range_start_bound.as_str()),
        AccountStickyKeySelection::Count(_) => None,
    };
    let preview_rows = query_account_sticky_key_recent_invocations(
        pool,
        account_id,
        &selected_keys,
        5,
        preview_range_start_bound,
    )
    .await?;
    let mut grouped_preview_rows: HashMap<
        String,
        Vec<crate::api::PromptCacheConversationInvocationPreviewResponse>,
    > = HashMap::new();
    for row in preview_rows {
        grouped_preview_rows
            .entry(row.sticky_key.clone())
            .or_default()
            .push(
                crate::api::PromptCacheConversationInvocationPreviewResponse {
                    id: row.id,
                    invoke_id: row.invoke_id,
                    occurred_at: row.occurred_at,
                    status: row.status,
                    failure_class: row.failure_class,
                    route_mode: row.route_mode,
                    model: row.model,
                    total_tokens: row.total_tokens,
                    cost: row.cost,
                    proxy_display_name: row.proxy_display_name,
                    upstream_account_id: row.upstream_account_id,
                    upstream_account_name: row.upstream_account_name,
                    endpoint: row.endpoint,
                    source: row.source,
                    input_tokens: row.input_tokens,
                    output_tokens: row.output_tokens,
                    cache_input_tokens: row.cache_input_tokens,
                    reasoning_tokens: row.reasoning_tokens,
                    reasoning_effort: row.reasoning_effort,
                    error_message: row.error_message,
                    downstream_status_code: row.downstream_status_code,
                    downstream_error_message: row.downstream_error_message,
                    failure_kind: row.failure_kind,
                    is_actionable: row.is_actionable.map(|value| value != 0),
                    response_content_encoding: row.response_content_encoding,
                    requested_service_tier: row.requested_service_tier,
                    service_tier: row.service_tier,
                    billing_service_tier: row.billing_service_tier,
                    t_req_read_ms: row.t_req_read_ms,
                    t_req_parse_ms: row.t_req_parse_ms,
                    t_upstream_connect_ms: row.t_upstream_connect_ms,
                    t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
                    t_upstream_stream_ms: row.t_upstream_stream_ms,
                    t_resp_parse_ms: row.t_resp_parse_ms,
                    t_persist_ms: row.t_persist_ms,
                    t_total_ms: row.t_total_ms,
                },
            );
    }
    for conversation in &mut conversations {
        conversation.recent_invocations = grouped_preview_rows
            .remove(&conversation.sticky_key)
            .unwrap_or_default();
    }

    Ok(AccountStickyKeysResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        selection_mode: selection.selection_mode(),
        selected_limit: selection.selected_limit(),
        selected_activity_hours: selection.selected_activity_hours(),
        implicit_filter: selection.implicit_filter(filtered_counts),
        conversations,
    })
}

async fn load_account_sticky_routes(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Vec<PoolStickyRouteRow>> {
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE account_id = ?1
        ORDER BY updated_at DESC, last_seen_at DESC, sticky_key ASC
        "#,
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

async fn query_account_sticky_key_aggregates(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selected_keys: &[String],
) -> Result<Vec<StickyKeyAggregateRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT sticky_key, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MIN(first_seen_at) AS created_at, \
             MAX(last_seen_at) AS last_activity_at \
         FROM upstream_sticky_key_hourly \
         WHERE upstream_account_id = ",
    );
    query.push_bind(account_id).push(" AND sticky_key IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(") GROUP BY sticky_key");

    query
        .build_query_as::<StickyKeyAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn query_account_sticky_key_events(
    pool: &Pool<Sqlite>,
    account_id: i64,
    range_start_bound: &str,
    selected_keys: &[String],
) -> Result<Vec<StickyKeyEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }
    const ACCOUNT_EXPR: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, 'unknown') AS status, COALESCE(total_tokens, 0) AS request_tokens, ",
    );
    query
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" AS sticky_key FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(ACCOUNT_EXPR)
        .push(" = ")
        .push_bind(account_id)
        .push(" AND ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" IN (");
    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(") ORDER BY sticky_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<StickyKeyEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn query_account_sticky_key_recent_invocations(
    pool: &Pool<Sqlite>,
    account_id: i64,
    selected_keys: &[String],
    limit_per_key: i64,
    range_start_bound: Option<&str>,
) -> Result<Vec<AccountStickyKeyInvocationPreviewRow>> {
    if selected_keys.is_empty() || limit_per_key <= 0 {
        return Ok(Vec::new());
    }

    let mut query =
        QueryBuilder::<Sqlite>::new("WITH ranked AS (SELECT id, invoke_id, occurred_at, ");
    query
        .push(crate::api::invocation_display_status_sql())
        .push(" AS status, ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(crate::api::INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(crate::api::INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(crate::api::INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(crate::api::INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(crate::api::INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(crate::api::INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(
            " AS response_content_encoding, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
                 THEN json_extract(payload, '$.requestedServiceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
                 THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
                 THEN json_extract(payload, '$.serviceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
                 THEN json_extract(payload, '$.service_tier') END AS service_tier, \
             ",
        )
        .push(crate::api::INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(
            " AS billing_service_tier, \
             t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
             t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ",
        )
        .push(crate::api::INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(crate::api::INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(crate::api::INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint, ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" AS sticky_key, ROW_NUMBER() OVER (PARTITION BY ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" ORDER BY occurred_at DESC, id DESC) AS row_number FROM codex_invocations WHERE ")
        .push(crate::api::INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" = ")
        .push_bind(account_id);

    if let Some(range_start_bound) = range_start_bound {
        query
            .push(" AND occurred_at >= ")
            .push_bind(range_start_bound);
    }

    query
        .push(" AND ")
        .push(crate::api::INVOCATION_STICKY_KEY_SQL)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }

    query
        .push(")) SELECT sticky_key, id, invoke_id, occurred_at, status, failure_class, route_mode, model, total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, reasoning_effort, error_message, downstream_status_code, downstream_error_message, failure_kind, is_actionable, proxy_display_name, upstream_account_id, upstream_account_name, response_content_encoding, requested_service_tier, service_tier, billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, endpoint FROM ranked WHERE row_number <= ")
        .push_bind(limit_per_key)
        .push(" ORDER BY sticky_key ASC, occurred_at DESC, id DESC");

    query
        .build_query_as::<AccountStickyKeyInvocationPreviewRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn prepare_pool_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: UpstreamAccountGroupMetadata,
    node_shunt_assignments: &UpstreamAccountNodeShuntAssignments,
) -> Result<Option<PoolResolvedAccount>> {
    let Some(crypto_key) = state.upstream_accounts.crypto_key.as_ref() else {
        return Ok(None);
    };
    let Some(encrypted_credentials) = row.encrypted_credentials.as_deref() else {
        return Ok(None);
    };
    let refresh_proxy_scope = required_account_forward_proxy_scope(
        row.group_name.as_deref(),
        group_metadata.bound_proxy_keys.clone(),
    )?;
    let forward_proxy_scope = resolve_account_forward_proxy_scope_from_assignments(
        row.id,
        row.group_name.as_deref(),
        &group_metadata,
        node_shunt_assignments,
    )?;
    let upstream_base_url =
        resolve_pool_account_upstream_base_url(row, &state.config.openai_upstream_base_url)?;
    let credentials = decrypt_credentials(crypto_key, encrypted_credentials)?;
    match credentials {
        StoredCredentials::ApiKey(value) => Ok(Some(PoolResolvedAccount {
            account_id: row.id,
            display_name: row.display_name.clone(),
            kind: row.kind.clone(),
            auth: PoolResolvedAuth::ApiKey {
                authorization: format!("Bearer {}", value.api_key),
            },
            group_name: row.group_name.clone(),
            bound_proxy_keys: group_metadata.bound_proxy_keys.clone(),
            forward_proxy_scope,
            group_upstream_429_retry_enabled: group_metadata.upstream_429_retry_enabled,
            group_upstream_429_max_retries: group_metadata.upstream_429_max_retries,
            fast_mode_rewrite_mode: effective_rule.fast_mode_rewrite_mode,
            upstream_base_url,
            routing_source: PoolRoutingSelectionSource::FreshAssignment,
        })),
        StoredCredentials::Oauth(mut value) => {
            let expires_at = row.token_expires_at.as_deref().and_then(parse_rfc3339_utc);
            let refresh_due = expires_at
                .map(|expires| {
                    expires
                        <= Utc::now()
                            + ChronoDuration::seconds(
                                state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                            )
                })
                .unwrap_or(true);
            if refresh_due {
                match refresh_oauth_tokens_for_required_scope(
                    state,
                    &refresh_proxy_scope,
                    &value.refresh_token,
                )
                .await
                {
                    Ok(response) => {
                        value.access_token = response.access_token;
                        if let Some(refresh_token) = response.refresh_token {
                            value.refresh_token = refresh_token;
                        }
                        if let Some(id_token) = response.id_token {
                            value.id_token = id_token;
                        }
                        value.token_type = response.token_type;
                        let token_expires_at = format_utc_iso(
                            Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                        );
                        persist_oauth_credentials(
                            &state.pool,
                            row.id,
                            crypto_key,
                            &value,
                            &token_expires_at,
                        )
                        .await?;
                    }
                    Err(err) if is_reauth_error(&err) => {
                        let err_text = err.to_string();
                        let now_iso = format_utc_iso(Utc::now());
                        sqlx::query(
                            r#"
                            UPDATE pool_upstream_accounts
                            SET status = ?2,
                                last_error = ?3,
                                last_error_at = ?4,
                                last_route_failure_at = ?4,
                                last_route_failure_kind = ?5,
                                cooldown_until = NULL,
                                consecutive_route_failures = consecutive_route_failures + 1,
                                temporary_route_failure_streak_started_at = NULL,
                                updated_at = ?4
                            WHERE id = ?1
                            "#,
                        )
                        .bind(row.id)
                        .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
                        .bind(&err_text)
                        .bind(&now_iso)
                        .bind(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
                        .execute(&state.pool)
                        .await?;
                        record_upstream_account_action(
                            &state.pool,
                            row.id,
                            UpstreamAccountActionPayload {
                                action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                                source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                                reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED),
                                reason_message: Some(&err_text),
                                http_status: None,
                                failure_kind: Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
                                invoke_id: None,
                                sticky_key: None,
                                occurred_at: &now_iso,
                            },
                        )
                        .await?;
                        return Ok(None);
                    }
                    Err(err) => {
                        let err_text = err.to_string();
                        let (disposition, reason_code, next_status, http_status, failure_kind) =
                            classify_sync_failure(&row.kind, &err_text);
                        match disposition {
                            UpstreamAccountFailureDisposition::HardUnavailable => {
                                let now_iso = format_utc_iso(Utc::now());
                                sqlx::query(
                                    r#"
                                    UPDATE pool_upstream_accounts
                                    SET status = ?2,
                                        last_error = ?3,
                                        last_error_at = ?4,
                                        last_route_failure_at = ?4,
                                        last_route_failure_kind = ?5,
                                        cooldown_until = NULL,
                                        consecutive_route_failures = consecutive_route_failures + 1,
                                        temporary_route_failure_streak_started_at = NULL,
                                        updated_at = ?4
                                    WHERE id = ?1
                                    "#,
                                )
                                .bind(row.id)
                                .bind(next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR))
                                .bind(&err_text)
                                .bind(&now_iso)
                                .bind(failure_kind)
                                .execute(&state.pool)
                                .await?;
                                record_upstream_account_action(
                                    &state.pool,
                                    row.id,
                                    UpstreamAccountActionPayload {
                                        action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                                        source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                                        reason_code: Some(reason_code),
                                        reason_message: Some(&err_text),
                                        http_status,
                                        failure_kind: Some(failure_kind),
                                        invoke_id: None,
                                        sticky_key: None,
                                        occurred_at: &now_iso,
                                    },
                                )
                                .await?;
                            }
                            UpstreamAccountFailureDisposition::RateLimited
                            | UpstreamAccountFailureDisposition::Retryable => {
                                apply_pool_route_cooldown_failure(
                                    &state.pool,
                                    row.id,
                                    None,
                                    &err_text,
                                    failure_kind,
                                    reason_code,
                                    http_status.unwrap_or(StatusCode::BAD_GATEWAY),
                                    5,
                                    None,
                                )
                                .await?;
                            }
                        }
                        return Ok(None);
                    }
                }
            }

            Ok(Some(PoolResolvedAccount {
                account_id: row.id,
                display_name: row.display_name.clone(),
                kind: row.kind.clone(),
                auth: PoolResolvedAuth::Oauth {
                    access_token: value.access_token,
                    chatgpt_account_id: row.chatgpt_account_id.clone(),
                },
                group_name: row.group_name.clone(),
                bound_proxy_keys: group_metadata.bound_proxy_keys,
                forward_proxy_scope,
                group_upstream_429_retry_enabled: group_metadata.upstream_429_retry_enabled,
                group_upstream_429_max_retries: group_metadata.upstream_429_max_retries,
                fast_mode_rewrite_mode: effective_rule.fast_mode_rewrite_mode,
                upstream_base_url,
                routing_source: PoolRoutingSelectionSource::FreshAssignment,
            }))
        }
    }
}

fn is_account_selectable_for_sticky_reuse(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    if !is_routing_eligible_account(row) || snapshot_exhausted {
        return false;
    }
    !account_has_active_cooldown(row.cooldown_until.as_deref(), now)
        || is_account_degraded_for_routing(row, snapshot_exhausted, now)
}

fn is_account_selectable_for_fresh_assignment(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    is_account_selectable_for_sticky_reuse(row, snapshot_exhausted, now)
        && !is_account_degraded_for_routing(row, snapshot_exhausted, now)
}

fn is_account_degraded_for_routing(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    is_routing_eligible_account(row)
        && !snapshot_exhausted
        && upstream_account_degraded_state_is_current(
            &row.status,
            row.cooldown_until.as_deref(),
            row.last_error_at.as_deref(),
            row.last_route_failure_at.as_deref(),
            row.last_route_failure_kind.as_deref(),
            row.last_action_reason_code.as_deref(),
            row.temporary_route_failure_streak_started_at.as_deref(),
            now,
        )
}

fn is_pool_account_routing_candidate(row: &UpstreamAccountRow) -> bool {
    row.provider == UPSTREAM_ACCOUNT_PROVIDER_CODEX
        && row.enabled != 0
        && row.status == UPSTREAM_ACCOUNT_STATUS_ACTIVE
}

fn is_routing_eligible_account(row: &UpstreamAccountRow) -> bool {
    is_pool_account_routing_candidate(row) && row.encrypted_credentials.is_some()
}

fn is_account_rate_limited_for_routing(row: &UpstreamAccountRow, snapshot_exhausted: bool) -> bool {
    if row.provider != UPSTREAM_ACCOUNT_PROVIDER_CODEX
        || row.enabled == 0
        || row.encrypted_credentials.is_none()
    {
        return false;
    }
    let quota_exhausted_hard_stop =
        route_failure_kind_is_quota_exhausted(row.last_route_failure_kind.as_deref());
    snapshot_exhausted
        || quota_exhausted_hard_stop
        || account_reason_is_rate_limited(row.last_action_reason_code.as_deref())
}

async fn load_account_routing_candidates(
    pool: &Pool<Sqlite>,
    excluded_ids: &HashSet<i64>,
) -> Result<Vec<AccountRoutingCandidateRow>> {
    let active_sticky_cutoff = format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES),
    );
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.plan_type
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS plan_type,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_window_minutes,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_window_minutes,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            account.local_primary_limit,
            account.local_secondary_limit,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance,
            account.last_selected_at,
            (
                SELECT COUNT(*)
                FROM pool_sticky_routes route
                WHERE route.account_id = account.id
                  AND route.last_seen_at >=
        "#,
    );
    query.push_bind(&active_sticky_cutoff).push(
        r#"
            ) AS active_sticky_conversations
        FROM pool_upstream_accounts account
        WHERE account.provider = 
        "#,
    );
    query
        .push_bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .push(" AND account.enabled = 1");
    if !excluded_ids.is_empty() {
        query.push(" AND account.id NOT IN (");
        {
            let mut separated = query.separated(", ");
            for account_id in excluded_ids {
                separated.push_bind(account_id);
            }
        }
        query.push(")");
    }
    query.push(" ORDER BY account.id ASC");

    query
        .build_query_as::<AccountRoutingCandidateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn load_account_routing_candidate(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<AccountRoutingCandidateRow>> {
    sqlx::query_as::<_, AccountRoutingCandidateRow>(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.plan_type
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS plan_type,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_window_minutes,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_window_minutes,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            account.local_primary_limit,
            account.local_secondary_limit,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance,
            account.last_selected_at,
            (
                SELECT COUNT(*)
                FROM pool_sticky_routes route
                WHERE route.account_id = account.id
                  AND route.last_seen_at >= ?2
            ) AS active_sticky_conversations
        FROM pool_upstream_accounts account
        WHERE account.id = ?1
        "#,
    )
    .bind(account_id)
    .bind(format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES),
    ))
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn compare_routing_candidates(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
) -> std::cmp::Ordering {
    compare_routing_candidates_at(lhs, rhs, Utc::now())
}

fn compare_routing_candidates_at(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
    now: DateTime<Utc>,
) -> std::cmp::Ordering {
    let lhs_capacity = lhs.capacity_profile();
    let rhs_capacity = rhs.capacity_profile();
    let lhs_over_soft_limit = lhs.effective_load() > lhs_capacity.soft_limit;
    let rhs_over_soft_limit = rhs.effective_load() > rhs_capacity.soft_limit;
    let lhs_score = lhs.scarcity_score(now);
    let rhs_score = rhs.scarcity_score(now);
    lhs_over_soft_limit
        .cmp(&rhs_over_soft_limit)
        .then_with(|| {
            lhs_score
                .partial_cmp(&rhs_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.effective_load().cmp(&rhs.effective_load()))
        .then_with(|| lhs.last_selected_at.cmp(&rhs.last_selected_at))
        .then_with(|| lhs.id.cmp(&rhs.id))
}

pub(crate) async fn record_account_selected(pool: &Pool<Sqlite>, account_id: i64) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_selected_at = ?2,
            updated_at = ?2
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn record_compact_support_observation(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    reason: Option<&str>,
) -> Result<()> {
    if !matches!(
        status,
        COMPACT_SUPPORT_STATUS_SUPPORTED | COMPACT_SUPPORT_STATUS_UNSUPPORTED
    ) {
        return Ok(());
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET compact_support_status = ?2,
            compact_support_observed_at = ?3,
            compact_support_reason = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(now_iso)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

async fn apply_pool_route_cooldown_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    error_message: &str,
    failure_kind: &str,
    reason_code: &str,
    http_status: StatusCode,
    base_secs: i64,
    invoke_id: Option<&str>,
) -> Result<()> {
    let row = load_upstream_account_row(pool, account_id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    let now = Utc::now();
    let continuing_temporary_streak = row.consecutive_route_failures > 0
        && route_failure_kind_is_temporary(row.last_route_failure_kind.as_deref());
    let next_failures = if continuing_temporary_streak {
        row.consecutive_route_failures.max(0) + 1
    } else {
        1
    };
    let streak_started_at = if continuing_temporary_streak {
        row.temporary_route_failure_streak_started_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .or_else(|| {
                row.last_route_failure_at
                    .as_deref()
                    .and_then(parse_rfc3339_utc)
            })
            .unwrap_or(now)
    } else {
        now
    };
    let should_start_cooldown = next_failures >= POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD
        || now.signed_duration_since(streak_started_at).num_seconds()
            >= POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS;
    let exponent = (next_failures - 1).clamp(0, 5) as u32;
    let cooldown_secs =
        (base_secs * (1_i64 << exponent)).min(POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS);
    let now_iso = format_utc_iso(now);
    let streak_started_at_iso = format_utc_iso(streak_started_at);
    let cooldown_until =
        should_start_cooldown.then(|| format_utc_iso(now + ChronoDuration::seconds(cooldown_secs)));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = ?4,
            last_route_failure_at = ?4,
            last_route_failure_kind = ?5,
            cooldown_until = ?6,
            consecutive_route_failures = ?7,
            temporary_route_failure_streak_started_at = ?8,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(error_message)
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(cooldown_until)
    .bind(next_failures)
    .bind(streak_started_at_iso)
    .execute(pool)
    .await?;
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: if should_start_cooldown {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED
            } else {
                UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE
            },
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
            reason_code: Some(reason_code),
            reason_message: Some(error_message),
            http_status: Some(http_status),
            failure_kind: Some(failure_kind),
            invoke_id,
            sticky_key,
            occurred_at: &now_iso,
        },
    )
    .await?;
    Ok(())
}

async fn load_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
) -> Result<Option<PoolStickyRouteRow>> {
    sqlx::query_as::<_, PoolStickyRouteRow>(
        r#"
        SELECT sticky_key, account_id, created_at, updated_at, last_seen_at
        FROM pool_sticky_routes
        WHERE sticky_key = ?1
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn upsert_sticky_route(
    pool: &Pool<Sqlite>,
    sticky_key: &str,
    account_id: i64,
    now_iso: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (
            sticky_key, account_id, created_at, updated_at, last_seen_at
        ) VALUES (?1, ?2, ?3, ?3, ?3)
        ON CONFLICT(sticky_key) DO UPDATE SET
            account_id = excluded.account_id,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn delete_sticky_route(pool: &Pool<Sqlite>, sticky_key: &str) -> Result<()> {
    sqlx::query("DELETE FROM pool_sticky_routes WHERE sticky_key = ?1")
        .bind(sticky_key)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(deprecated)]
fn encrypt_secret_value(key: &[u8; 32], value: &str) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), value.as_bytes())
        .map_err(|err| anyhow!("failed to encrypt secret: {err}"))?;
    serde_json::to_string(&EncryptedCredentialsPayload {
        v: 1,
        nonce: BASE64_STANDARD.encode(nonce),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    })
    .context("failed to encode encrypted secret payload")
}

#[allow(deprecated)]
fn decrypt_secret_value(key: &[u8; 32], payload: &str) -> Result<String> {
    let payload: EncryptedCredentialsPayload =
        serde_json::from_str(payload).context("failed to decode encrypted secret payload")?;
    if payload.v != 1 {
        bail!(
            "unsupported encrypted secret payload version: {}",
            payload.v
        );
    }
    let nonce = BASE64_STANDARD
        .decode(payload.nonce)
        .context("failed to decode secret nonce")?;
    let ciphertext = BASE64_STANDARD
        .decode(payload.ciphertext)
        .context("failed to decode secret ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let plaintext = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| anyhow!("failed to decrypt secret: {err}"))?;
    String::from_utf8(plaintext).context("failed to decode decrypted secret")
}
