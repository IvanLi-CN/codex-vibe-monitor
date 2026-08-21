use super::*;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
pub(crate) const CACHE_HIT_PROTECTION_MIN_INPUT_TOKENS: u64 = 3_840;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheHitOverflowMode {
    Queue,
    Reroute,
}

impl CacheHitOverflowMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Reroute => "reroute",
        }
    }

    fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("reroute") => Self::Reroute,
            _ => Self::Queue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CacheHitProtectionSettings {
    pub(crate) enabled: bool,
    pub(crate) low_hit_rate_threshold_percent: u8,
    pub(crate) overflow_mode: CacheHitOverflowMode,
}

impl CacheHitProtectionSettings {
    pub(crate) fn into_response(self) -> CacheHitProtectionSettingsResponse {
        CacheHitProtectionSettingsResponse {
            enabled: self.enabled,
            low_hit_rate_threshold_percent: self.low_hit_rate_threshold_percent,
            overflow_mode: self.overflow_mode.as_str().to_string(),
            minimum_input_tokens: CACHE_HIT_PROTECTION_MIN_INPUT_TOKENS,
        }
    }
}

pub(crate) fn resolve_cache_hit_protection_settings(
    row: &PoolRoutingSettingsRow,
) -> CacheHitProtectionSettings {
    CacheHitProtectionSettings {
        enabled: row.cache_hit_protection_enabled.unwrap_or_default() != 0,
        low_hit_rate_threshold_percent: row
            .cache_hit_low_rate_threshold_percent
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| (1..=100).contains(value))
            .unwrap_or(10),
        overflow_mode: CacheHitOverflowMode::parse(row.cache_hit_overflow_mode.as_deref()),
    }
}

pub(crate) fn merge_cache_hit_protection_settings(
    current: CacheHitProtectionSettings,
    patch: Option<&UpdateCacheHitProtectionSettingsRequest>,
) -> Result<CacheHitProtectionSettings, (StatusCode, String)> {
    let Some(patch) = patch else {
        return Ok(current);
    };
    let low_hit_rate_threshold_percent = patch
        .low_hit_rate_threshold_percent
        .unwrap_or(current.low_hit_rate_threshold_percent);
    if !(1..=100).contains(&low_hit_rate_threshold_percent) {
        return Err((
            StatusCode::BAD_REQUEST,
            "cacheHitProtection.lowHitRateThresholdPercent must be between 1 and 100".to_string(),
        ));
    }
    let overflow_mode = match patch.overflow_mode.as_deref() {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "queue" => CacheHitOverflowMode::Queue,
            "reroute" => CacheHitOverflowMode::Reroute,
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "cacheHitProtection.overflowMode must be queue or reroute".to_string(),
                ));
            }
        },
        None => current.overflow_mode,
    };
    Ok(CacheHitProtectionSettings {
        enabled: patch.enabled.unwrap_or(current.enabled),
        low_hit_rate_threshold_percent,
        overflow_mode,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveRequestStreamingSettings {
    pub(crate) enabled: bool,
    pub(crate) treatment_percent: u8,
}

impl LiveRequestStreamingSettings {
    pub(crate) fn into_response(self) -> LiveRequestStreamingSettingsResponse {
        LiveRequestStreamingSettingsResponse {
            enabled: self.enabled,
            treatment_percent: self.treatment_percent,
        }
    }
}

pub(crate) fn resolve_live_request_streaming_settings(
    row: &PoolRoutingSettingsRow,
) -> LiveRequestStreamingSettings {
    LiveRequestStreamingSettings {
        enabled: row.live_request_streaming_enabled.unwrap_or_default() != 0,
        treatment_percent: row
            .live_request_streaming_treatment_percent
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| *value <= 100)
            .unwrap_or(50),
    }
}

pub(crate) fn merge_live_request_streaming_settings(
    current: LiveRequestStreamingSettings,
    patch: Option<&UpdateLiveRequestStreamingSettingsRequest>,
) -> Result<LiveRequestStreamingSettings, (StatusCode, String)> {
    let Some(patch) = patch else {
        return Ok(current);
    };
    let treatment_percent = patch.treatment_percent.unwrap_or(current.treatment_percent);
    if treatment_percent > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            "liveRequestStreaming.treatmentPercent must be between 0 and 100".to_string(),
        ));
    }
    Ok(LiveRequestStreamingSettings {
        enabled: patch.enabled.unwrap_or(current.enabled),
        treatment_percent,
    })
}

pub(crate) fn pool_routing_timeouts_from_config(
    config: &AppConfig,
) -> PoolRoutingTimeoutSettingsResolved {
    PoolRoutingTimeoutSettingsResolved {
        default_first_byte_timeout: config.request_timeout,
        default_send_timeout: config.openai_proxy_handshake_timeout,
        request_read_timeout: config.openai_proxy_request_read_timeout,
        responses_first_byte_timeout: config.pool_upstream_responses_attempt_timeout,
        compact_first_byte_timeout: config.openai_proxy_compact_handshake_timeout,
        image_first_byte_timeout: config.openai_proxy_image_handshake_timeout,
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
        image_first_byte_timeout: row
            .image_first_byte_timeout_secs
            .and_then(|value| u64::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.image_first_byte_timeout),
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

pub(crate) fn resolve_pool_request_compression_settings_from_row(
    row: &PoolRoutingSettingsRow,
) -> PoolRoutingRequestCompressionSettingsResolved {
    PoolRoutingRequestCompressionSettingsResolved {
        algorithm: row
            .request_compression_algorithm
            .as_deref()
            .map(RequestCompressionAlgorithm::from_str)
            .unwrap_or_default(),
        level_preset: decode_request_compression_level_preset(
            row.request_compression_level_preset.as_deref(),
        ),
    }
}

pub(crate) fn pool_routing_timeouts_response(
    resolved: PoolRoutingTimeoutSettingsResolved,
) -> PoolRoutingTimeoutSettingsResponse {
    PoolRoutingTimeoutSettingsResponse {
        responses_first_byte_timeout_secs: resolved.responses_first_byte_timeout.as_secs(),
        compact_first_byte_timeout_secs: resolved.compact_first_byte_timeout.as_secs(),
        image_first_byte_timeout_secs: resolved.image_first_byte_timeout.as_secs(),
        responses_stream_timeout_secs: resolved.responses_stream_timeout.as_secs(),
        compact_stream_timeout_secs: resolved.compact_stream_timeout.as_secs(),
    }
}

pub(crate) fn normalize_optional_timeout_override_secs(
    value: &OptionalField<u64>,
    field_name: &str,
) -> Result<Option<Option<i64>>, (StatusCode, String)> {
    match value {
        OptionalField::Missing => Ok(None),
        OptionalField::Null => Ok(Some(None)),
        OptionalField::Value(value) => {
            let normalized = normalize_pool_routing_timeout_secs(Some(*value), field_name)?
                .and_then(|value| i64::try_from(value).ok());
            Ok(Some(normalized))
        }
    }
}

pub(crate) fn normalize_timeout_override_secs_from_i64(value: Option<i64>) -> Option<u64> {
    value
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
}

pub(crate) fn routing_timeout_settings_from_columns(
    responses_first_byte_timeout_secs: Option<i64>,
    compact_first_byte_timeout_secs: Option<i64>,
    image_first_byte_timeout_secs: Option<i64>,
    responses_stream_timeout_secs: Option<i64>,
    compact_stream_timeout_secs: Option<i64>,
) -> Option<RoutingTimeoutSettings> {
    let settings = RoutingTimeoutSettings {
        responses_first_byte_timeout_secs: normalize_timeout_override_secs_from_i64(
            responses_first_byte_timeout_secs,
        ),
        compact_first_byte_timeout_secs: normalize_timeout_override_secs_from_i64(
            compact_first_byte_timeout_secs,
        ),
        image_first_byte_timeout_secs: normalize_timeout_override_secs_from_i64(
            image_first_byte_timeout_secs,
        ),
        responses_stream_timeout_secs: normalize_timeout_override_secs_from_i64(
            responses_stream_timeout_secs,
        ),
        compact_stream_timeout_secs: normalize_timeout_override_secs_from_i64(
            compact_stream_timeout_secs,
        ),
    };
    if settings.responses_first_byte_timeout_secs.is_none()
        && settings.compact_first_byte_timeout_secs.is_none()
        && settings.image_first_byte_timeout_secs.is_none()
        && settings.responses_stream_timeout_secs.is_none()
        && settings.compact_stream_timeout_secs.is_none()
    {
        None
    } else {
        Some(settings)
    }
}

pub(crate) fn routing_timeout_overrides_from_settings(
    settings: Option<&RoutingTimeoutSettings>,
) -> RoutingTimeoutOverridesResolved {
    let duration = |value: Option<u64>| value.map(Duration::from_secs);
    let Some(settings) = settings else {
        return RoutingTimeoutOverridesResolved::default();
    };
    RoutingTimeoutOverridesResolved {
        responses_first_byte_timeout: duration(settings.responses_first_byte_timeout_secs),
        compact_first_byte_timeout: duration(settings.compact_first_byte_timeout_secs),
        image_first_byte_timeout: duration(settings.image_first_byte_timeout_secs),
        responses_stream_timeout: duration(settings.responses_stream_timeout_secs),
        compact_stream_timeout: duration(settings.compact_stream_timeout_secs),
    }
}

pub(crate) fn resolve_effective_routing_timeout_settings(
    root: PoolRoutingTimeoutSettingsResolved,
    group: Option<&RoutingTimeoutSettings>,
    account: Option<&RoutingTimeoutSettings>,
    conversation: Option<&RoutingTimeoutSettings>,
) -> (
    RoutingTimeoutSettings,
    RoutingTimeoutFieldSources,
    PoolRoutingTimeoutSettingsResolved,
) {
    let mut overrides = RoutingTimeoutSettings::default();
    let mut effective = RoutingTimeoutSettings {
        responses_first_byte_timeout_secs: Some(root.responses_first_byte_timeout.as_secs()),
        compact_first_byte_timeout_secs: Some(root.compact_first_byte_timeout.as_secs()),
        image_first_byte_timeout_secs: Some(root.image_first_byte_timeout.as_secs()),
        responses_stream_timeout_secs: Some(root.responses_stream_timeout.as_secs()),
        compact_stream_timeout_secs: Some(root.compact_stream_timeout.as_secs()),
    };
    let mut sources = RoutingTimeoutFieldSources {
        responses_first_byte_timeout_secs: "root".to_string(),
        compact_first_byte_timeout_secs: "root".to_string(),
        image_first_byte_timeout_secs: "root".to_string(),
        responses_stream_timeout_secs: "root".to_string(),
        compact_stream_timeout_secs: "root".to_string(),
    };

    let mut apply = |source: &str, settings: Option<&RoutingTimeoutSettings>| {
        let Some(settings) = settings else {
            return;
        };
        if let Some(value) = settings.responses_first_byte_timeout_secs {
            overrides.responses_first_byte_timeout_secs = Some(value);
            effective.responses_first_byte_timeout_secs = Some(value);
            sources.responses_first_byte_timeout_secs = source.to_string();
        }
        if let Some(value) = settings.compact_first_byte_timeout_secs {
            overrides.compact_first_byte_timeout_secs = Some(value);
            effective.compact_first_byte_timeout_secs = Some(value);
            sources.compact_first_byte_timeout_secs = source.to_string();
        }
        if let Some(value) = settings.image_first_byte_timeout_secs {
            overrides.image_first_byte_timeout_secs = Some(value);
            effective.image_first_byte_timeout_secs = Some(value);
            sources.image_first_byte_timeout_secs = source.to_string();
        }
        if let Some(value) = settings.responses_stream_timeout_secs {
            overrides.responses_stream_timeout_secs = Some(value);
            effective.responses_stream_timeout_secs = Some(value);
            sources.responses_stream_timeout_secs = source.to_string();
        }
        if let Some(value) = settings.compact_stream_timeout_secs {
            overrides.compact_stream_timeout_secs = Some(value);
            effective.compact_stream_timeout_secs = Some(value);
            sources.compact_stream_timeout_secs = source.to_string();
        }
    };

    apply("group", group);
    apply("account", account);
    apply("conversation", conversation);
    let resolved = root.with_overrides(routing_timeout_overrides_from_settings(Some(&overrides)));
    (effective, sources, resolved)
}

pub(crate) async fn load_effective_request_path_timeouts_for_account(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    account_id: i64,
    prompt_cache_key: Option<&str>,
) -> Result<(
    RoutingTimeoutSettings,
    RoutingTimeoutFieldSources,
    PoolRoutingTimeoutSettingsResolved,
)> {
    let root = resolve_pool_routing_timeouts(pool, config).await?;

    #[derive(Debug, FromRow)]
    struct AccountTimeoutRow {
        group_name: Option<String>,
        policy_responses_first_byte_timeout_secs: Option<i64>,
        policy_compact_first_byte_timeout_secs: Option<i64>,
        policy_image_first_byte_timeout_secs: Option<i64>,
        policy_responses_stream_timeout_secs: Option<i64>,
        policy_compact_stream_timeout_secs: Option<i64>,
    }

    let account_row = sqlx::query_as::<_, AccountTimeoutRow>(
        r#"
        SELECT
            group_name,
            policy_responses_first_byte_timeout_secs,
            policy_compact_first_byte_timeout_secs,
            policy_image_first_byte_timeout_secs,
            policy_responses_stream_timeout_secs,
            policy_compact_stream_timeout_secs
        FROM pool_upstream_accounts
        WHERE id = ?1 AND COALESCE(deleted_at, '') = ''
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let Some(account_row) = account_row else {
        let (effective, sources, resolved) =
            resolve_effective_routing_timeout_settings(root, None, None, None);
        return Ok((effective, sources, resolved));
    };

    let group_settings = if let Some(group_name) = account_row
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        #[derive(Debug, FromRow)]
        struct GroupTimeoutRow {
            policy_responses_first_byte_timeout_secs: Option<i64>,
            policy_compact_first_byte_timeout_secs: Option<i64>,
            policy_image_first_byte_timeout_secs: Option<i64>,
            policy_responses_stream_timeout_secs: Option<i64>,
            policy_compact_stream_timeout_secs: Option<i64>,
        }
        sqlx::query_as::<_, GroupTimeoutRow>(
            r#"
            SELECT
                policy_responses_first_byte_timeout_secs,
                policy_compact_first_byte_timeout_secs,
                policy_image_first_byte_timeout_secs,
                policy_responses_stream_timeout_secs,
                policy_compact_stream_timeout_secs
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            LIMIT 1
            "#,
        )
        .bind(group_name)
        .fetch_optional(pool)
        .await?
        .and_then(|row| {
            routing_timeout_settings_from_columns(
                row.policy_responses_first_byte_timeout_secs,
                row.policy_compact_first_byte_timeout_secs,
                row.policy_image_first_byte_timeout_secs,
                row.policy_responses_stream_timeout_secs,
                row.policy_compact_stream_timeout_secs,
            )
        })
    } else {
        None
    };

    let account_settings = routing_timeout_settings_from_columns(
        account_row.policy_responses_first_byte_timeout_secs,
        account_row.policy_compact_first_byte_timeout_secs,
        account_row.policy_image_first_byte_timeout_secs,
        account_row.policy_responses_stream_timeout_secs,
        account_row.policy_compact_stream_timeout_secs,
    );

    let conversation_settings = if let Some(prompt_cache_key) = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        #[derive(Debug, FromRow)]
        struct ConversationTimeoutRow {
            responses_first_byte_timeout_secs: Option<i64>,
            compact_first_byte_timeout_secs: Option<i64>,
            image_first_byte_timeout_secs: Option<i64>,
            responses_stream_timeout_secs: Option<i64>,
            compact_stream_timeout_secs: Option<i64>,
        }
        sqlx::query_as::<_, ConversationTimeoutRow>(
            r#"
            SELECT
                responses_first_byte_timeout_secs,
                compact_first_byte_timeout_secs,
                image_first_byte_timeout_secs,
                responses_stream_timeout_secs,
                compact_stream_timeout_secs
            FROM prompt_cache_conversation_bindings
            WHERE prompt_cache_key = ?1
            LIMIT 1
            "#,
        )
        .bind(prompt_cache_key)
        .fetch_optional(pool)
        .await?
        .and_then(|row| {
            routing_timeout_settings_from_columns(
                row.responses_first_byte_timeout_secs,
                row.compact_first_byte_timeout_secs,
                row.image_first_byte_timeout_secs,
                row.responses_stream_timeout_secs,
                row.compact_stream_timeout_secs,
            )
        })
    } else {
        None
    };

    let (effective, sources, resolved) = resolve_effective_routing_timeout_settings(
        root,
        group_settings.as_ref(),
        account_settings.as_ref(),
        conversation_settings.as_ref(),
    );
    Ok((effective, sources, resolved))
}

pub(crate) async fn load_effective_request_path_timeouts_for_group(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    group_name: Option<&str>,
) -> Result<(
    RoutingTimeoutSettings,
    RoutingTimeoutFieldSources,
    PoolRoutingTimeoutSettingsResolved,
)> {
    let root = resolve_pool_routing_timeouts(pool, config).await?;
    let group_settings =
        if let Some(group_name) = group_name.map(str::trim).filter(|value| !value.is_empty()) {
            #[derive(Debug, FromRow)]
            struct GroupTimeoutRow {
                policy_responses_first_byte_timeout_secs: Option<i64>,
                policy_compact_first_byte_timeout_secs: Option<i64>,
                policy_image_first_byte_timeout_secs: Option<i64>,
                policy_responses_stream_timeout_secs: Option<i64>,
                policy_compact_stream_timeout_secs: Option<i64>,
            }

            sqlx::query_as::<_, GroupTimeoutRow>(
                r#"
            SELECT
                policy_responses_first_byte_timeout_secs,
                policy_compact_first_byte_timeout_secs,
                policy_image_first_byte_timeout_secs,
                policy_responses_stream_timeout_secs,
                policy_compact_stream_timeout_secs
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            LIMIT 1
            "#,
            )
            .bind(group_name)
            .fetch_optional(pool)
            .await?
            .and_then(|row| {
                routing_timeout_settings_from_columns(
                    row.policy_responses_first_byte_timeout_secs,
                    row.policy_compact_first_byte_timeout_secs,
                    row.policy_image_first_byte_timeout_secs,
                    row.policy_responses_stream_timeout_secs,
                    row.policy_compact_stream_timeout_secs,
                )
            })
        } else {
            None
        };

    let (effective, sources, resolved) =
        resolve_effective_routing_timeout_settings(root, group_settings.as_ref(), None, None);
    Ok((effective, sources, resolved))
}

pub(crate) async fn load_effective_request_path_timeouts_for_group_and_conversation(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    group_name: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> Result<(
    RoutingTimeoutSettings,
    RoutingTimeoutFieldSources,
    PoolRoutingTimeoutSettingsResolved,
)> {
    let root = resolve_pool_routing_timeouts(pool, config).await?;
    let group_settings =
        if let Some(group_name) = group_name.map(str::trim).filter(|value| !value.is_empty()) {
            #[derive(Debug, FromRow)]
            struct GroupTimeoutRow {
                policy_responses_first_byte_timeout_secs: Option<i64>,
                policy_compact_first_byte_timeout_secs: Option<i64>,
                policy_image_first_byte_timeout_secs: Option<i64>,
                policy_responses_stream_timeout_secs: Option<i64>,
                policy_compact_stream_timeout_secs: Option<i64>,
            }

            sqlx::query_as::<_, GroupTimeoutRow>(
                r#"
            SELECT
                policy_responses_first_byte_timeout_secs,
                policy_compact_first_byte_timeout_secs,
                policy_image_first_byte_timeout_secs,
                policy_responses_stream_timeout_secs,
                policy_compact_stream_timeout_secs
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            LIMIT 1
            "#,
            )
            .bind(group_name)
            .fetch_optional(pool)
            .await?
            .and_then(|row| {
                routing_timeout_settings_from_columns(
                    row.policy_responses_first_byte_timeout_secs,
                    row.policy_compact_first_byte_timeout_secs,
                    row.policy_image_first_byte_timeout_secs,
                    row.policy_responses_stream_timeout_secs,
                    row.policy_compact_stream_timeout_secs,
                )
            })
        } else {
            None
        };

    let conversation_settings = if let Some(prompt_cache_key) = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        #[derive(Debug, FromRow)]
        struct ConversationTimeoutRow {
            responses_first_byte_timeout_secs: Option<i64>,
            compact_first_byte_timeout_secs: Option<i64>,
            image_first_byte_timeout_secs: Option<i64>,
            responses_stream_timeout_secs: Option<i64>,
            compact_stream_timeout_secs: Option<i64>,
        }

        sqlx::query_as::<_, ConversationTimeoutRow>(
            r#"
            SELECT
                responses_first_byte_timeout_secs,
                compact_first_byte_timeout_secs,
                image_first_byte_timeout_secs,
                responses_stream_timeout_secs,
                compact_stream_timeout_secs
            FROM prompt_cache_conversation_bindings
            WHERE prompt_cache_key = ?1
            LIMIT 1
            "#,
        )
        .bind(prompt_cache_key)
        .fetch_optional(pool)
        .await?
        .and_then(|row| {
            routing_timeout_settings_from_columns(
                row.responses_first_byte_timeout_secs,
                row.compact_first_byte_timeout_secs,
                row.image_first_byte_timeout_secs,
                row.responses_stream_timeout_secs,
                row.compact_stream_timeout_secs,
            )
        })
    } else {
        None
    };

    let (effective, sources, resolved) = resolve_effective_routing_timeout_settings(
        root,
        group_settings.as_ref(),
        None,
        conversation_settings.as_ref(),
    );
    Ok((effective, sources, resolved))
}

pub(crate) async fn load_pool_routing_settings(
    pool: &Pool<Sqlite>,
) -> Result<PoolRoutingSettingsRow> {
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
            image_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs,
            request_compression_algorithm,
            request_compression_level_preset,
            codex_imagegen_rewrite_mode,
            available_models_json,
            available_models_mode,
            default_first_byte_timeout_secs,
            upstream_handshake_timeout_secs,
            request_read_timeout_secs
            ,cache_hit_protection_enabled
            ,cache_hit_low_rate_threshold_percent
            ,cache_hit_overflow_mode
            ,live_request_streaming_enabled
            ,live_request_streaming_treatment_percent
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
    let request_compression = resolve_pool_request_compression_settings_from_row(row);
    let (available_models, available_models_invalid) =
        parse_string_array_json_with_invalid(row.available_models_json.as_deref());
    PoolRoutingSettingsResponse {
        writes_enabled: true,
        api_key_configured: row
            .encrypted_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        masked_api_key: row.masked_api_key.clone(),
        maintenance: resolve_pool_routing_maintenance_settings(row, &state.config).into_response(),
        request_compression_algorithm: request_compression.algorithm,
        request_compression_level_preset: request_compression.level_preset,
        codex_imagegen_rewrite_mode: row
            .codex_imagegen_rewrite_mode
            .as_deref()
            .map(CodexImagegenRewriteMode::from_str)
            .unwrap_or(CodexImagegenRewriteMode::KeepOriginal),
        available_models: if available_models_invalid {
            Vec::new()
        } else {
            available_models
        },
        available_models_mode: if available_models_invalid {
            AvailableModelsMode::Allowlist
        } else {
            AvailableModelsMode::from_str(row.available_models_mode.as_deref())
        },
        timeouts: pool_routing_timeouts_response(timeouts),
        cache_hit_protection: resolve_cache_hit_protection_settings(row).into_response(),
        live_request_streaming: resolve_live_request_streaming_settings(row).into_response(),
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
        generation: 0,
        invalidated: false,
        #[cfg(test)]
        sqlite_data_version: 0,
        api_key,
        request_compression: resolve_pool_request_compression_settings_from_row(row),
        timeouts: resolve_pool_routing_timeouts_from_row(row, &state.config),
        cache_hit_protection: resolve_cache_hit_protection_settings(row),
        live_request_streaming: resolve_live_request_streaming_settings(row),
        model_routing: PoolModelRoutingRuntimeCache::default(),
        prompt_route_cache: Arc::new(std::sync::Mutex::new(PoolRoutingPromptRouteCache::default())),
        sticky_route_cache: Arc::new(std::sync::Mutex::new(PoolRoutingStickyRouteCache::default())),
    })
}

pub(crate) async fn refresh_pool_routing_runtime_cache(
    state: &AppState,
) -> Result<PoolRoutingRuntimeCache> {
    let _model_cache_write_guard = state.pool_model_routing_cache_write_lock.lock().await;
    refresh_pool_routing_runtime_cache_locked(state).await
}

async fn refresh_pool_routing_runtime_cache_locked(
    state: &AppState,
) -> Result<PoolRoutingRuntimeCache> {
    let row = load_pool_routing_settings_seeded(&state.pool, &state.config).await?;
    let mut cache = build_pool_routing_runtime_cache(state, &row)?;
    let mut model_routing = build_pool_model_routing_runtime_cache(&state.pool).await?;
    #[cfg(test)]
    {
        cache.sqlite_data_version = pool_routing_sqlite_data_version(state).await?;
    }
    let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
    cache.generation = runtime_cache
        .as_ref()
        .map(|previous| previous.generation.saturating_add(1))
        .unwrap_or(1);
    model_routing.generation = runtime_cache
        .as_ref()
        .map(|previous| previous.model_routing.generation.saturating_add(1))
        .unwrap_or(1);
    cache.model_routing = model_routing;
    *runtime_cache = Some(cache.clone());
    Ok(cache)
}

pub(crate) async fn load_pool_routing_runtime_cache(
    state: &AppState,
) -> Result<PoolRoutingRuntimeCache> {
    Ok(load_pool_routing_runtime_cache_with_status(state).await?.0)
}

/// Returns the immutable routing snapshot and whether this request observed an
/// already-warm snapshot. Cold loads are serialized by the shared write lock.
pub(crate) async fn load_pool_routing_runtime_cache_with_status(
    state: &AppState,
) -> Result<(PoolRoutingRuntimeCache, bool)> {
    let warm_cache = {
        let runtime_cache = state.pool_routing_runtime_cache.lock().await;
        runtime_cache
            .as_ref()
            .filter(|cache| !cache.invalidated)
            .cloned()
    };
    #[cfg(test)]
    if let Some(cache) = warm_cache
        && pool_routing_sqlite_data_version(state).await? == cache.sqlite_data_version
    {
        return Ok((cache, true));
    }
    #[cfg(not(test))]
    if let Some(cache) = warm_cache {
        return Ok((cache, true));
    }

    // The first request after cold start performs one shared load. Later
    // requests only clone the immutable runtime snapshot above.
    let _model_cache_write_guard = state.pool_model_routing_cache_write_lock.lock().await;
    let warm_cache = {
        let runtime_cache = state.pool_routing_runtime_cache.lock().await;
        runtime_cache
            .as_ref()
            .filter(|cache| !cache.invalidated)
            .cloned()
    };
    #[cfg(test)]
    if let Some(cache) = warm_cache
        && pool_routing_sqlite_data_version(state).await? == cache.sqlite_data_version
    {
        return Ok((cache, true));
    }
    #[cfg(not(test))]
    if let Some(cache) = warm_cache {
        return Ok((cache, true));
    }
    Ok((
        refresh_pool_routing_runtime_cache_locked(state).await?,
        false,
    ))
}

/// Invalidates the current immutable routing snapshot without doing I/O in the
/// failure path. The next route selection performs the serialized cold rebuild
/// and receives a strictly newer generation.
pub(crate) async fn invalidate_pool_routing_runtime_cache(state: &AppState) {
    let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
    if let Some(cache) = runtime_cache.as_mut() {
        cache.invalidated = true;
    }
}

#[cfg(test)]
async fn pool_routing_sqlite_data_version(state: &AppState) -> Result<i64> {
    // Test fixtures intentionally perform direct SQL writes. Keep one observer
    // connection out of that writer pool so SQLite's per-connection data_version
    // reliably detects those writes without changing production behavior.
    let mut observer = state.pool_routing_test_data_version_connection.lock().await;
    if observer.is_none() {
        *observer = Some(
            state
                .pool
                .acquire()
                .await
                .context("acquire SQLite routing test observer")?,
        );
    }
    sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(
            &mut **observer
                .as_mut()
                .expect("routing test observer is initialized"),
        )
        .await
        .context("read SQLite data version for routing test snapshot")
}

pub(crate) struct PoolRoutingSettingsUpdate<'a> {
    pub(crate) crypto_key: Option<&'a [u8; 32]>,
    pub(crate) api_key: Option<&'a str>,
    pub(crate) request_compression_algorithm: Option<RequestCompressionAlgorithm>,
    pub(crate) request_compression_level_preset: Option<RequestCompressionLevelPreset>,
    pub(crate) codex_imagegen_rewrite_mode: Option<CodexImagegenRewriteMode>,
    pub(crate) available_models: Option<&'a [String]>,
    pub(crate) available_models_mode: Option<AvailableModelsMode>,
    pub(crate) timeout_updates: Option<&'a UpdatePoolRoutingTimeoutSettingsRequest>,
    pub(crate) maintenance_settings: Option<PoolRoutingMaintenanceSettings>,
    pub(crate) cache_hit_protection: Option<CacheHitProtectionSettings>,
    pub(crate) live_request_streaming: Option<LiveRequestStreamingSettings>,
}

pub(crate) async fn save_pool_routing_settings(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    update: PoolRoutingSettingsUpdate<'_>,
) -> Result<PoolRoutingSettingsRow, (StatusCode, String)> {
    let current = load_pool_routing_settings_seeded(pool, config)
        .await
        .map_err(internal_error_tuple)?;
    let encrypted_api_key = match update.api_key {
        Some(api_key) => {
            let crypto_key = update.crypto_key.ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "pool routing secret storage is unavailable".to_string(),
                )
            })?;
            Some(encrypt_secret_value(crypto_key, api_key).map_err(internal_error_tuple)?)
        }
        None => current.encrypted_api_key.clone(),
    };
    let masked_api_key = match update.api_key {
        Some(api_key) => Some(mask_api_key(api_key)),
        None => current.masked_api_key.clone(),
    };
    let (primary_sync_interval_secs, secondary_sync_interval_secs, priority_available_account_cap) =
        if let Some(maintenance_settings) = update.maintenance_settings {
            (
                Some(
                    i64::try_from(maintenance_settings.primary_sync_interval_secs)
                        .map_err(|err| internal_error_tuple(anyhow!(err)))?,
                ),
                Some(
                    i64::try_from(maintenance_settings.secondary_sync_interval_secs)
                        .map_err(|err| internal_error_tuple(anyhow!(err)))?,
                ),
                Some(
                    i64::try_from(maintenance_settings.priority_available_account_cap)
                        .map_err(|err| internal_error_tuple(anyhow!(err)))?,
                ),
            )
        } else {
            (
                current.primary_sync_interval_secs,
                current.secondary_sync_interval_secs,
                current.priority_available_account_cap,
            )
        };
    let responses_first_byte_timeout_secs = update
        .timeout_updates
        .and_then(|value| value.responses_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.responses_first_byte_timeout_secs);
    let compact_first_byte_timeout_secs = update
        .timeout_updates
        .and_then(|value| value.compact_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.compact_first_byte_timeout_secs);
    let image_first_byte_timeout_secs = update
        .timeout_updates
        .and_then(|value| value.image_first_byte_timeout_secs)
        .map(|value| value as i64)
        .or(current.image_first_byte_timeout_secs);
    let responses_stream_timeout_secs = update
        .timeout_updates
        .and_then(|value| value.responses_stream_timeout_secs)
        .map(|value| value as i64)
        .or(current.responses_stream_timeout_secs);
    let compact_stream_timeout_secs = update
        .timeout_updates
        .and_then(|value| value.compact_stream_timeout_secs)
        .map(|value| value as i64)
        .or(current.compact_stream_timeout_secs);
    let request_compression_algorithm = update
        .request_compression_algorithm
        .map(|value| value.as_str().to_string())
        .or(current.request_compression_algorithm.clone());
    let request_compression_level_preset = update
        .request_compression_level_preset
        .map(|value| value.as_str().to_string())
        .or(current.request_compression_level_preset.clone());
    let codex_imagegen_rewrite_mode = update
        .codex_imagegen_rewrite_mode
        .map(|value| value.as_str().to_string())
        .or(current.codex_imagegen_rewrite_mode.clone());
    let available_models_json = update
        .available_models
        .map(|models| serde_json::to_string(models).unwrap_or_else(|_| "[]".to_string()))
        .or(current.available_models_json.clone())
        .unwrap_or_else(|| "[]".to_string());
    let available_models_mode = update
        .available_models_mode
        .map(|value| match value {
            AvailableModelsMode::Allowlist => "allowlist".to_string(),
            AvailableModelsMode::Denylist => "denylist".to_string(),
        })
        .or(current.available_models_mode.clone())
        .unwrap_or_else(|| "denylist".to_string());
    let default_first_byte_timeout_secs = current.default_first_byte_timeout_secs;
    let upstream_handshake_timeout_secs = current.upstream_handshake_timeout_secs;
    let request_read_timeout_secs = current.request_read_timeout_secs;
    let cache_hit_protection = update
        .cache_hit_protection
        .unwrap_or_else(|| resolve_cache_hit_protection_settings(&current));
    let live_request_streaming_updated = update.live_request_streaming.is_some();
    let live_request_streaming = update
        .live_request_streaming
        .unwrap_or_else(|| resolve_live_request_streaming_settings(&current));
    let now_iso = format_utc_iso(Utc::now());

    let mut tx = pool.begin().await.map_err(internal_error_tuple)?;
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
            image_first_byte_timeout_secs = ?9,
            responses_stream_timeout_secs = ?10,
            compact_stream_timeout_secs = ?11,
            request_compression_algorithm = ?12,
            request_compression_level_preset = ?13,
            codex_imagegen_rewrite_mode = ?14,
            -- Model policy columns are updated below with field-local writes.
            available_models_json = CASE WHEN ?15 IS NULL THEN available_models_json ELSE available_models_json END,
            available_models_mode = CASE WHEN ?16 IS NULL THEN available_models_mode ELSE available_models_mode END,
            default_first_byte_timeout_secs = ?17,
            upstream_handshake_timeout_secs = ?18,
            request_read_timeout_secs = ?19,
            updated_at = ?20
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
    .bind(image_first_byte_timeout_secs)
    .bind(responses_stream_timeout_secs)
    .bind(compact_stream_timeout_secs)
    .bind(request_compression_algorithm)
    .bind(request_compression_level_preset)
    .bind(codex_imagegen_rewrite_mode)
    .bind(available_models_json)
    .bind(available_models_mode)
    .bind(default_first_byte_timeout_secs)
    .bind(upstream_handshake_timeout_secs)
    .bind(request_read_timeout_secs)
    .bind(now_iso)
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;

    match (update.available_models, update.available_models_mode) {
        (Some(models), Some(mode)) => {
            sqlx::query(
                "UPDATE pool_routing_settings SET available_models_json = ?2, available_models_mode = ?3 WHERE id = ?1",
            )
            .bind(POOL_SETTINGS_SINGLETON_ID)
            .bind(serde_json::to_string(models).unwrap_or_else(|_| "[]".to_string()))
            .bind(match mode {
                AvailableModelsMode::Allowlist => "allowlist",
                AvailableModelsMode::Denylist => "denylist",
            })
            .execute(&mut *tx)
            .await
            .map_err(internal_error_tuple)?;
        }
        (Some(models), None) => {
            sqlx::query(
                "UPDATE pool_routing_settings SET available_models_json = ?2 WHERE id = ?1",
            )
            .bind(POOL_SETTINGS_SINGLETON_ID)
            .bind(serde_json::to_string(models).unwrap_or_else(|_| "[]".to_string()))
            .execute(&mut *tx)
            .await
            .map_err(internal_error_tuple)?;
        }
        (None, Some(mode)) => {
            sqlx::query(
                "UPDATE pool_routing_settings SET available_models_mode = ?2 WHERE id = ?1",
            )
            .bind(POOL_SETTINGS_SINGLETON_ID)
            .bind(match mode {
                AvailableModelsMode::Allowlist => "allowlist",
                AvailableModelsMode::Denylist => "denylist",
            })
            .execute(&mut *tx)
            .await
            .map_err(internal_error_tuple)?;
        }
        (None, None) => {}
    }

    if update.cache_hit_protection.is_some() {
        sqlx::query(
            "UPDATE pool_routing_settings SET cache_hit_protection_enabled = ?2, cache_hit_low_rate_threshold_percent = ?3, cache_hit_overflow_mode = ?4, updated_at = ?5 WHERE id = ?1",
        )
        .bind(POOL_SETTINGS_SINGLETON_ID)
        .bind(if cache_hit_protection.enabled { 1_i64 } else { 0_i64 })
        .bind(i64::from(cache_hit_protection.low_hit_rate_threshold_percent))
        .bind(cache_hit_protection.overflow_mode.as_str())
        .bind(format_utc_iso(Utc::now()))
        .execute(&mut *tx)
        .await
        .map_err(internal_error_tuple)?;
    }

    if live_request_streaming_updated {
        sqlx::query(
            "UPDATE pool_routing_settings SET live_request_streaming_enabled = ?2, live_request_streaming_treatment_percent = ?3, updated_at = ?4 WHERE id = ?1",
        )
        .bind(POOL_SETTINGS_SINGLETON_ID)
        .bind(if live_request_streaming.enabled { 1_i64 } else { 0_i64 })
        .bind(i64::from(live_request_streaming.treatment_percent))
        .bind(format_utc_iso(Utc::now()))
        .execute(&mut *tx)
        .await
        .map_err(internal_error_tuple)?;
    }

    tx.commit().await.map_err(internal_error_tuple)?;
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
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) upstream_429_retry_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) image_tool_rewrite_mode: ImageToolRewriteMode,
    pub(crate) codex_imagegen_rewrite_mode: CodexImagegenRewriteMode,
    pub(crate) request_compression_algorithm: RequestCompressionAlgorithm,
    pub(crate) response_endpoint_capability: CapabilitySupport,
    pub(crate) chat_completions_capability: CapabilitySupport,
    pub(crate) image_endpoint_capability: CapabilitySupport,
    pub(crate) response_image_tool_capability: CapabilitySupport,
    pub(crate) codex_imagegen_capability: CapabilitySupport,
    pub(crate) standalone_search_capability: CapabilitySupport,
    pub(crate) upstream_base_url: Url,
    pub(crate) routing_source: PoolRoutingSelectionSource,
    pub(crate) sticky_affinity_generation: Option<i64>,
    pub(crate) routing_selection_audit: Option<PoolRoutingSelectionAudit>,
}

impl PoolResolvedAccount {
    pub(crate) fn upstream_route_key(&self) -> String {
        canonical_pool_upstream_route_key(&self.upstream_base_url)
    }

    pub(crate) fn effective_upstream_429_max_retries(&self) -> u8 {
        normalize_group_upstream_429_retry_metadata(
            self.upstream_429_retry_enabled,
            self.upstream_429_max_retries,
        )
    }

    pub(crate) fn with_sticky_affinity_generation(mut self, generation: Option<i64>) -> Self {
        self.sticky_affinity_generation = generation;
        self
    }

    pub(crate) fn with_routing_selection_audit(mut self, audit: PoolRoutingSelectionAudit) -> Self {
        self.routing_selection_audit = Some(audit);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSelectionScoreSnapshot {
    pub(crate) eligibility: String,
    pub(crate) route_binding_failure_penalty: i64,
    pub(crate) model_route_penalty: u8,
    pub(crate) model_route_penalty_code: String,
    pub(crate) routing_priority_rank: u8,
    pub(crate) capacity_lane: String,
    pub(crate) dispatch_state: String,
    pub(crate) secondary_reset_proximity_secs: Option<i64>,
    pub(crate) primary_reset_proximity_secs: Option<i64>,
    pub(crate) scarcity_score: String,
    pub(crate) effective_load: i64,
    pub(crate) last_selected_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSelectionAudit {
    pub(crate) selected_account_id: i64,
    pub(crate) selected_account_name: String,
    pub(crate) eligible_candidate_count: usize,
    pub(crate) winner_reason_code: String,
    pub(crate) compared_account_id: Option<i64>,
    pub(crate) compared_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) selected_score: Option<PoolRoutingSelectionScoreSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) compared_score: Option<PoolRoutingSelectionScoreSnapshot>,
    pub(crate) excluded_candidates: Vec<PoolRoutingSelectionAuditExcludedCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingSelectionAuditExcludedCandidate {
    pub(crate) account_id: i64,
    pub(crate) account_name: String,
    pub(crate) reason_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingSelectionSource {
    StickyReuse,
    FreshAssignment,
}

impl PoolRoutingSelectionSource {
    pub(crate) fn as_persisted_str(self) -> &'static str {
        match self {
            Self::StickyReuse => "stickyReuse",
            Self::FreshAssignment => "freshAssignment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingCandidateEligibility {
    Assignable,
    SoftDegraded,
    HardBlocked,
}

impl PoolRoutingCandidateEligibility {
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Assignable => 0,
            Self::SoftDegraded => 1,
            Self::HardBlocked => 2,
        }
    }

    pub(crate) fn as_persisted_str(self) -> &'static str {
        match self {
            Self::Assignable => "assignable",
            Self::SoftDegraded => "softDegraded",
            Self::HardBlocked => "hardBlocked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRoutingCandidateCapacityLane {
    Primary,
    Overflow,
}

impl PoolRoutingCandidateCapacityLane {
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Primary => 0,
            Self::Overflow => 1,
        }
    }

    pub(crate) fn as_persisted_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Overflow => "overflow",
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
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::ReadyOnOwnedNode => 0,
            Self::ReadyAfterMigration => 1,
            Self::RetryOriginalNode => 2,
            Self::HardBlocked => 3,
        }
    }

    pub(crate) fn as_persisted_str(self) -> &'static str {
        match self {
            Self::ReadyOnOwnedNode => "readyOnOwnedNode",
            Self::ReadyAfterMigration => "readyAfterMigration",
            Self::RetryOriginalNode => "retryOriginalNode",
            Self::HardBlocked => "hardBlocked",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingCandidateScore {
    pub(crate) eligibility: PoolRoutingCandidateEligibility,
    pub(crate) route_binding_failure_penalty: i64,
    pub(crate) model_route_penalty: u8,
    pub(crate) routing_priority_rank: u8,
    pub(crate) capacity_lane: PoolRoutingCandidateCapacityLane,
    pub(crate) dispatch_state: PoolRoutingCandidateDispatchState,
    pub(crate) single_account_rotation_enabled: bool,
    pub(crate) secondary_reset_proximity_secs: Option<i64>,
    pub(crate) primary_reset_proximity_secs: Option<i64>,
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
    NoCandidate(PoolRoutingNoCandidateAudit),
    BlockedByPolicy(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingNoCandidateAudit {
    pub(crate) terminal_reason_code: String,
    pub(crate) candidate_count: usize,
    pub(crate) eligible_candidate_count: usize,
    pub(crate) reservation_conflict_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_eligible_at: Option<String>,
    pub(crate) excluded_reason_counts: std::collections::BTreeMap<String, usize>,
    pub(crate) candidates: Vec<PoolRoutingNoCandidateAuditCandidate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolRoutingNoCandidateAuditCandidate {
    pub(crate) account_id: i64,
    pub(crate) account_name: String,
    pub(crate) reason_code: String,
}

impl PoolRoutingNoCandidateAudit {
    pub(crate) fn no_eligible() -> Self {
        Self {
            terminal_reason_code: "noEligibleCandidate".to_string(),
            candidate_count: 0,
            eligible_candidate_count: 0,
            reservation_conflict_count: 0,
            next_eligible_at: None,
            excluded_reason_counts: std::collections::BTreeMap::new(),
            candidates: Vec::new(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_request_streaming_settings_default_to_disabled() {
        let current = LiveRequestStreamingSettings {
            enabled: false,
            treatment_percent: 50,
        };
        let default = merge_live_request_streaming_settings(current.clone(), None)
            .expect("default settings should be valid");
        assert_eq!(default, current);

        let updated = merge_live_request_streaming_settings(
            current,
            Some(&UpdateLiveRequestStreamingSettingsRequest {
                enabled: Some(true),
                treatment_percent: Some(50),
            }),
        )
        .expect("valid settings should merge");
        assert!(updated.enabled);
        assert_eq!(updated.treatment_percent, 50);
    }

    #[test]
    fn live_request_streaming_settings_reject_invalid_treatment() {
        let current = LiveRequestStreamingSettings {
            enabled: false,
            treatment_percent: 50,
        };
        let invalid_percent = merge_live_request_streaming_settings(
            current,
            Some(&UpdateLiveRequestStreamingSettingsRequest {
                enabled: None,
                treatment_percent: Some(101),
            }),
        )
        .expect_err("treatment percent above 100 should be rejected");
        assert_eq!(invalid_percent.0, StatusCode::BAD_REQUEST);
    }
}
