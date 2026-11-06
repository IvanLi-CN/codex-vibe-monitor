use super::*;

static POOL_ROUTING_SETTINGS_WRITE_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

pub(crate) async fn get_pool_routing_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PoolRoutingSettingsResponse>, (StatusCode, String)> {
    let row = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(build_pool_routing_settings_response(
        state.as_ref(),
        &row,
    )))
}

pub(crate) async fn update_pool_routing_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<UpdatePoolRoutingSettingsRequest>,
) -> Result<Json<PoolRoutingSettingsResponse>, (StatusCode, String)> {
    let _write_guard = POOL_ROUTING_SETTINGS_WRITE_LOCK.lock().await;
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let current = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    let merged_maintenance = merge_pool_routing_maintenance_settings(
        resolve_pool_routing_maintenance_settings(&current, &state.config),
        payload.maintenance.as_ref(),
    );
    validate_pool_routing_maintenance_settings(merged_maintenance)?;
    let current_cache_hit_protection = resolve_cache_hit_protection_settings(&current);
    let merged_cache_hit_protection = merge_cache_hit_protection_settings(
        current_cache_hit_protection,
        payload.cache_hit_protection.as_ref(),
    )?;
    let cache_hit_protection_reset_reason = payload
        .cache_hit_protection
        .is_some()
        .then_some({
            if !merged_cache_hit_protection.enabled {
                Some("cache_hit_protection_disabled")
            } else if merged_cache_hit_protection.low_hit_rate_threshold_percent
                != current_cache_hit_protection.low_hit_rate_threshold_percent
            {
                Some("cache_hit_threshold_changed")
            } else {
                None
            }
        })
        .flatten();

    let api_key = payload
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| normalize_required_secret(value, "apiKey"))
        .transpose()?;
    let timeout_updates = payload
        .timeouts
        .map(|timeouts| {
            Ok(UpdatePoolRoutingTimeoutSettingsRequest {
                responses_first_byte_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.responses_first_byte_timeout_secs,
                    "responsesFirstByteTimeoutSecs",
                )?,
                compact_first_byte_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.compact_first_byte_timeout_secs,
                    "compactFirstByteTimeoutSecs",
                )?,
                image_first_byte_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.image_first_byte_timeout_secs,
                    "imageFirstByteTimeoutSecs",
                )?,
                responses_stream_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.responses_stream_timeout_secs,
                    "responsesStreamTimeoutSecs",
                )?,
                compact_stream_timeout_secs: normalize_pool_routing_timeout_secs(
                    timeouts.compact_stream_timeout_secs,
                    "compactStreamTimeoutSecs",
                )?,
            })
        })
        .transpose()?;
    let request_compression_algorithm = payload
        .request_compression_algorithm
        .as_deref()
        .map(|value| normalize_request_compression_algorithm(Some(value)))
        .transpose()?;
    let oauth_request_compression_algorithm = payload
        .oauth_request_compression_algorithm
        .as_deref()
        .map(|value| normalize_request_compression_algorithm(Some(value)))
        .transpose()?;
    let request_compression_level_preset = payload
        .request_compression_level_preset
        .as_deref()
        .map(|value| normalize_request_compression_level_preset(Some(value)))
        .transpose()?;
    let codex_imagegen_rewrite_mode = payload
        .codex_imagegen_rewrite_mode
        .as_deref()
        .map(|value| super::sync::normalize_codex_imagegen_rewrite_mode(Some(value)))
        .transpose()?;
    let available_models_mode = match payload.available_models_mode.as_deref() {
        Some(value) => Some(match value.trim().to_ascii_lowercase().as_str() {
            "allowlist" => AvailableModelsMode::Allowlist,
            "denylist" => AvailableModelsMode::Denylist,
            _ => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "availableModelsMode must be allowlist or denylist".to_string(),
                ));
            }
        }),
        None if payload.available_models.is_some() => Some(AvailableModelsMode::Allowlist),
        None => None,
    };
    let available_models = payload
        .available_models
        .as_ref()
        .map(|models| normalize_available_models(Some(models.clone()), "availableModels"))
        .transpose()?;
    let reserve_fallback_model_policy = payload
        .reserve_fallback_model_policy
        .as_ref()
        .map(normalize_reserve_model_policy)
        .transpose()?;
    let crypto_key = if api_key.is_some() {
        Some(state.upstream_accounts.require_crypto_key()?)
    } else {
        None
    };
    if api_key.is_some()
        || timeout_updates.is_some()
        || request_compression_algorithm.is_some()
        || oauth_request_compression_algorithm.is_some()
        || request_compression_level_preset.is_some()
        || codex_imagegen_rewrite_mode.is_some()
        || available_models.is_some()
        || available_models_mode.is_some()
        || payload.maintenance.is_some()
        || payload.cache_hit_protection.is_some()
        || payload.priority_handoff_admission_enabled.is_some()
        || payload.automatic_reserve_fallback_enabled.is_some()
        || reserve_fallback_model_policy.is_some()
    {
        save_pool_routing_settings(
            &state.pool,
            &state.config,
            PoolRoutingSettingsUpdate {
                crypto_key,
                api_key: api_key.as_deref(),
                request_compression_algorithm,
                oauth_request_compression_algorithm,
                request_compression_level_preset,
                codex_imagegen_rewrite_mode,
                available_models: available_models.as_deref(),
                available_models_mode,
                timeout_updates: timeout_updates.as_ref(),
                maintenance_settings: payload.maintenance.as_ref().map(|_| merged_maintenance),
                cache_hit_protection: payload
                    .cache_hit_protection
                    .as_ref()
                    .map(|_| merged_cache_hit_protection),
                priority_handoff_admission_enabled: payload.priority_handoff_admission_enabled,
                automatic_reserve_fallback_enabled: payload.automatic_reserve_fallback_enabled,
                reserve_fallback_model_policy: reserve_fallback_model_policy.as_ref(),
            },
        )
        .await?;
        if let Some(reason) = cache_hit_protection_reset_reason {
            clear_cache_hit_protection_state(&state.pool, reason)
                .await
                .map_err(internal_error_tuple)?;
            state
                .subscription_hub
                .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
            publish_pool_routing_availability(state.as_ref());
        }
        if let Err(err) =
            publish_account_effective_routing_rules_changed(state.as_ref(), None, &[]).await
        {
            warn!(
                ?err,
                "pool routing settings committed but effective routing rule publication failed"
            );
            invalidate_dashboard_activity_snapshots_with_accounts(
                state.dashboard_activity_snapshot_cache.as_ref(),
                "account_effective_routing_rules_publication_failed",
            )
            .await;
        }
        if let Some(enabled) = payload.priority_handoff_admission_enabled {
            // The priority gate is a process-local routing authority. Apply a successful settings
            // write directly even when an unrelated cache refresh is unavailable.
            set_priority_handoff_admission_enabled(enabled);
        }
    }
    let updated = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(build_pool_routing_settings_response(
        state.as_ref(),
        &updated,
    )))
}

pub(crate) async fn get_upstream_account_sticky_keys(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Query(params): Query<AccountStickyKeysQuery>,
) -> Result<Json<AccountStickyKeysResponse>, (StatusCode, String)> {
    let exists = load_upstream_account_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .is_some();
    if !exists {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    let selection = resolve_sticky_key_selection(&params)?;
    let response = build_account_sticky_keys_response(&state.pool, id, selection)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(response))
}
