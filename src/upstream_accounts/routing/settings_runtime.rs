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

pub(crate) fn normalize_pool_routing_timeout_secs(
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

pub(crate) fn resolve_pool_routing_timeouts_from_row(
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

pub(crate) fn pool_routing_timeouts_response(
    resolved: PoolRoutingTimeoutSettingsResolved,
) -> PoolRoutingTimeoutSettingsResponse {
    PoolRoutingTimeoutSettingsResponse {
        responses_first_byte_timeout_secs: resolved.responses_first_byte_timeout.as_secs(),
        compact_first_byte_timeout_secs: resolved.compact_first_byte_timeout.as_secs(),
        responses_stream_timeout_secs: resolved.responses_stream_timeout.as_secs(),
        compact_stream_timeout_secs: resolved.compact_stream_timeout.as_secs(),
    }
}

pub(crate) async fn load_pool_routing_settings(pool: &Pool<Sqlite>) -> Result<PoolRoutingSettingsRow> {
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

pub(crate) fn resolve_pool_routing_maintenance_settings(
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

pub(crate) fn build_pool_routing_settings_response(
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

pub(crate) fn validate_pool_routing_maintenance_settings(
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

pub(crate) fn merge_pool_routing_maintenance_settings(
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

pub(crate) async fn load_pool_routing_settings_seeded(
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

pub(crate) fn build_pool_routing_runtime_cache(
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
    let row = load_pool_routing_settings_seeded(&state.pool, &state.config).await?;
    let cache = build_pool_routing_runtime_cache(state, &row)?;
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

pub(crate) async fn save_pool_routing_settings(
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

pub(crate) async fn save_pool_routing_api_key(
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

pub(crate) async fn save_pool_routing_maintenance_settings(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingCandidateEligibility {
    Assignable,
    SoftDegraded,
    HardBlocked,
}

impl PoolRoutingCandidateEligibility {
    fn rank(self) -> u8 {
        match self {
            Self::Assignable => 0,
            Self::SoftDegraded => 1,
            Self::HardBlocked => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingCandidateCapacityLane {
    Primary,
    Overflow,
}

impl PoolRoutingCandidateCapacityLane {
    fn rank(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Overflow => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingCandidateDispatchState {
    ReadyOnOwnedNode,
    ReadyAfterMigration,
    RetryOriginalNode,
    HardBlocked,
}

impl PoolRoutingCandidateDispatchState {
    fn rank(self) -> u8 {
        match self {
            Self::ReadyOnOwnedNode => 0,
            Self::ReadyAfterMigration => 1,
            Self::RetryOriginalNode => 2,
            Self::HardBlocked => 3,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingCandidateScore {
    pub(crate) eligibility: PoolRoutingCandidateEligibility,
    pub(crate) routing_priority_rank: u8,
    pub(crate) capacity_lane: PoolRoutingCandidateCapacityLane,
    pub(crate) dispatch_state: PoolRoutingCandidateDispatchState,
    pub(crate) scarcity_score: f64,
    pub(crate) effective_load: i64,
    pub(crate) last_selected_at: Option<String>,
    pub(crate) account_id: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolAssignedBlockedAccount {
    pub(crate) account: PoolResolvedAccount,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) enum PoolAccountResolution {
    Resolved(PoolResolvedAccount),
    AssignedBlocked(PoolAssignedBlockedAccount),
    RateLimited,
    DegradedOnly,
    Unavailable,
    NoCandidate,
    BlockedByPolicy(String),
}

#[derive(Debug, Clone)]
pub(crate) enum PoolAccountGroupProxyRoutingReadiness {
    Ready(UpstreamAccountGroupMetadata),
    Blocked(String),
}

#[allow(deprecated)]
pub(crate) fn encrypt_secret_value(key: &[u8; 32], value: &str) -> Result<String> {
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
pub(crate) fn decrypt_secret_value(key: &[u8; 32], payload: &str) -> Result<String> {
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
