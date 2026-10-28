use super::*;

pub(crate) async fn update_upstream_account_inner(
    state: &AppState,
    id: i64,
    payload: UpdateUpstreamAccountRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let mut row = load_upstream_account_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let is_api_key_account = row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX;
    let invalidate_runtime_outcomes =
        account_update_requests_manual_recovery(&payload) || payload.routing_rule.is_some();
    if is_api_key_account
        && (payload.group_name.is_some()
            || payload.group_bound_proxy_keys.is_some()
            || payload.group_node_shunt_enabled.is_some()
            || payload.group_single_account_rotation_enabled.is_some()
            || payload.group_note.is_some()
            || payload.concurrency_limit.is_some()
            || payload.is_mother == Some(true))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "API Key accounts cannot use groups, group policies, or mother-account routing"
                .to_string(),
        ));
    }
    let was_fresh_routable = is_account_selectable_for_fresh_assignment(&row, false, Utc::now());
    let clear_hard_failure_after_update = is_api_key_account
        && account_update_requests_manual_recovery(&payload)
        && route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        );
    let policy_priority_tier = payload
        .routing_rule
        .as_ref()
        .and_then(|rule| match &rule.priority_tier {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        })
        .map(|value| normalize_tag_priority_tier(Some(value)).map(|tier| tier.as_str().to_string()))
        .transpose()?;
    let policy_fast_mode_rewrite_mode = payload
        .routing_rule
        .as_ref()
        .and_then(|rule| match &rule.fast_mode_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        })
        .map(|value| {
            normalize_tag_fast_mode_rewrite_mode(Some(value)).map(|mode| mode.as_str().to_string())
        })
        .transpose()?;
    let policy_image_tool_rewrite_mode = payload
        .routing_rule
        .as_ref()
        .and_then(|rule| match &rule.image_tool_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        })
        .map(|value| {
            super::sync::normalize_upstream_image_tool_rewrite_mode(Some(value))
                .map(|mode| mode.as_str().to_string())
        })
        .transpose()?;
    let policy_codex_imagegen_rewrite_mode = payload
        .routing_rule
        .as_ref()
        .and_then(|rule| match &rule.codex_imagegen_rewrite_mode {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        })
        .map(|value| {
            super::sync::normalize_codex_imagegen_rewrite_mode(Some(value))
                .map(|mode| mode.as_str().to_string())
        })
        .transpose()?;
    let policy_request_compression_algorithm = payload
        .routing_rule
        .as_ref()
        .and_then(|rule| match &rule.request_compression_algorithm {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        })
        .map(|value| {
            normalize_request_compression_algorithm(Some(value))
                .map(|mode| mode.as_str().to_string())
        })
        .transpose()?;
    let policy_oauth_request_compression_algorithm = payload
        .routing_rule
        .as_ref()
        .and_then(|rule| match &rule.oauth_request_compression_algorithm {
            OptionalField::Value(value) => Some(value.as_str()),
            OptionalField::Missing | OptionalField::Null => None,
        })
        .map(|value| {
            normalize_request_compression_algorithm(Some(value))
                .map(|mode| mode.as_str().to_string())
        })
        .transpose()?;
    // Empty tagIds remains a compatibility no-op. Manual tag mutation is removed,
    // so account edits must not clear preserved system tags.
    if let Some(values) = payload.tag_ids.as_ref() {
        reject_manual_tag_ids(values)?;
    }
    let previous_display_name = row.display_name.clone();
    let previous_email = row.email.clone();
    let previous_group_name = row.group_name.clone();
    let requested_group_note = payload
        .group_note
        .clone()
        .map(|value| normalize_optional_text(Some(value)));
    let normalized_group_concurrency_limit =
        normalize_concurrency_limit(payload.concurrency_limit, "concurrencyLimit")?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        requested_group_note.clone().flatten(),
        payload.group_note.is_some(),
        payload.group_bound_proxy_keys.clone(),
        payload.group_bound_proxy_keys.is_some(),
        normalized_group_concurrency_limit,
        payload.concurrency_limit.is_some(),
        payload.group_node_shunt_enabled,
        payload.group_node_shunt_enabled.is_some(),
        payload.group_single_account_rotation_enabled,
        payload.group_single_account_rotation_enabled.is_some(),
    );

    let requested_display_name = match payload.display_name.clone() {
        Some(display_name) => Some(normalize_required_display_name(&display_name)?),
        None => None,
    };
    match payload.email.clone() {
        OptionalField::Missing => {}
        OptionalField::Null => {
            row.email = None;
        }
        OptionalField::Value(value) => {
            row.email = normalize_optional_email(Some(value), "email")?;
        }
    }
    row.display_name = resolve_display_name_after_email_change(
        Some(previous_display_name.as_str()),
        requested_display_name.as_deref(),
        previous_email.as_deref(),
        row.email.as_deref(),
    )
    .unwrap_or(previous_display_name);
    if is_api_key_account {
        row.group_name = None;
        row.is_mother = 0;
    } else {
        if let Some(group_name) = payload.group_name.clone() {
            row.group_name = Some(normalize_upstream_account_group_name(Some(group_name)));
        }
        if row.group_name.is_none() {
            row.group_name = Some(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME.to_string());
        }
    }
    if let Some(note) = payload.note {
        row.note = normalize_optional_text(Some(note));
    }
    if is_api_key_account {
        match payload.upstream_base_url {
            OptionalField::Missing => {}
            OptionalField::Null => {
                row.upstream_base_url = None;
            }
            OptionalField::Value(value) => {
                row.upstream_base_url = normalize_optional_upstream_base_url(Some(value))?;
            }
        }
    }
    if let Some(enabled) = payload.enabled {
        row.enabled = if enabled { 1 } else { 0 };
    }
    if let Some(is_mother) = payload.is_mother {
        row.is_mother = if is_mother { 1 } else { 0 };
    }

    if is_api_key_account {
        if let Some(api_key) = payload.api_key {
            let api_key = normalize_required_secret(&api_key, "apiKey")?;
            row.masked_api_key = Some(mask_api_key(&api_key));
            row.encrypted_credentials = Some(
                encrypt_credentials(
                    crypto_key,
                    &StoredCredentials::ApiKey(StoredApiKeyCredentials { api_key }),
                )
                .map_err(internal_error_tuple)?,
            );
        }
        if payload.local_primary_limit.is_some() {
            row.local_primary_limit = payload.local_primary_limit;
        }
        if payload.local_secondary_limit.is_some() {
            row.local_secondary_limit = payload.local_secondary_limit;
        }
        if payload.local_limit_unit.is_some() {
            row.local_limit_unit = Some(normalize_limit_unit(payload.local_limit_unit));
        }
        validate_local_limits(row.local_primary_limit, row.local_secondary_limit)?;
    }
    if !is_api_key_account {
        validate_group_note_target(row.group_name.as_deref(), requested_group_note.is_some())?;
    }
    let resolved_group_binding = if !is_api_key_account
        && (payload.group_name.is_some()
            || payload.group_bound_proxy_keys.is_some()
            || payload.group_node_shunt_enabled.is_some()
            || payload.group_single_account_rotation_enabled.is_some())
    {
        Some(
            resolve_required_group_proxy_binding_for_write(
                state,
                row.group_name.clone(),
                payload.group_bound_proxy_keys.clone(),
                payload.group_node_shunt_enabled,
            )
            .await?,
        )
    } else {
        None
    };
    if let Some(resolved_group_binding) = resolved_group_binding.as_ref() {
        row.group_name = Some(resolved_group_binding.group_name.clone());
    }
    let next_bound_proxy_keys_json = match payload.bound_proxy_keys {
        OptionalField::Missing if is_api_key_account => {
            let existing = decode_group_bound_proxy_keys_json(row.bound_proxy_keys_json.as_deref());
            let effective = if existing.is_empty() {
                vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
            } else {
                existing
            };
            Some(encode_group_bound_proxy_keys_json(&effective).map_err(internal_error_tuple)?)
        }
        OptionalField::Missing => row.bound_proxy_keys_json.clone(),
        OptionalField::Null if is_api_key_account => {
            return Err((
                StatusCode::BAD_REQUEST,
                "API Key accounts require at least one bound proxy node".to_string(),
            ));
        }
        OptionalField::Null => None,
        OptionalField::Value(values) => {
            let normalized = normalize_bound_proxy_keys(values);
            if normalized.is_empty() {
                if is_api_key_account {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "API Key accounts require at least one bound proxy node".to_string(),
                    ));
                }
                None
            } else {
                let canonical = canonicalize_forward_proxy_bound_keys(state, &normalized)
                    .await
                    .map_err(internal_error_tuple)?;
                let has_selectable = {
                    let manager = state.forward_proxy.lock().await;
                    manager.has_selectable_bound_proxy_keys(&canonical)
                };
                if !has_selectable {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        if is_api_key_account {
                            "API Key accounts require at least one available proxy node".to_string()
                        } else {
                            "select at least one available proxy node or clear account proxy bindings"
                                .to_string()
                        },
                    ));
                }
                Some(encode_group_bound_proxy_keys_json(&canonical).map_err(internal_error_tuple)?)
            }
        }
    };
    let normalize_capability_override =
        |field_name: &str, value: &str| -> Result<String, (StatusCode, String)> {
            match CapabilitySupport::from_str(value) {
                CapabilitySupport::Supported | CapabilitySupport::Unsupported => {
                    Ok(CapabilitySupport::from_str(value).as_str().to_string())
                }
                CapabilitySupport::Unknown => Err((
                    StatusCode::BAD_REQUEST,
                    format!("{field_name} must be supported, unsupported, or null"),
                )),
            }
        };
    let response_endpoint_capability_override = match &payload.response_endpoint_capability_override
    {
        OptionalField::Missing => row.policy_response_endpoint_capability_override.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => Some(normalize_capability_override(
            "responseEndpointCapabilityOverride",
            value,
        )?),
    };
    let chat_completions_capability_override = match &payload.chat_completions_capability_override {
        OptionalField::Missing => row.policy_chat_completions_capability_override.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => Some(normalize_capability_override(
            "chatCompletionsCapabilityOverride",
            value,
        )?),
    };
    let image_endpoint_capability_override = match &payload.image_endpoint_capability_override {
        OptionalField::Missing => row.policy_image_endpoint_capability_override.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => Some(normalize_capability_override(
            "imageEndpointCapabilityOverride",
            value,
        )?),
    };
    let response_image_tool_capability_override =
        match &payload.response_image_tool_capability_override {
            OptionalField::Missing => row.policy_response_image_tool_capability_override.clone(),
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(normalize_capability_override(
                "responseImageToolCapabilityOverride",
                value,
            )?),
        };
    let codex_imagegen_capability_override = match &payload.codex_imagegen_capability_override {
        OptionalField::Missing => row.policy_codex_imagegen_capability_override.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => Some(normalize_capability_override(
            "codexImagegenCapabilityOverride",
            value,
        )?),
    };
    if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
        && !matches!(
            &payload.standalone_search_capability_override,
            OptionalField::Missing
        )
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "standaloneSearchCapabilityOverride is only supported for API key accounts".to_string(),
        ));
    }
    let standalone_search_capability_override = if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        None
    } else {
        match &payload.standalone_search_capability_override {
            OptionalField::Missing => row.policy_standalone_search_capability_override.clone(),
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(normalize_capability_override(
                "standaloneSearchCapabilityOverride",
                value,
            )?),
        }
    };
    let status_change_upstream_http_401 = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_http_401,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_http_401,
    };
    let status_change_upstream_http_402 = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_http_402,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_http_402,
    };
    let status_change_upstream_http_403 = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_http_403,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_http_403,
    };
    let status_change_reauth_required = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_reauth_required,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_reauth_required,
    };
    let status_change_upstream_http_429_rate_limit = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_http_429_rate_limit,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_http_429_rate_limit,
    };
    let status_change_upstream_http_429_quota_exhausted = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            )
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_http_429_quota_exhausted,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_http_429_quota_exhausted,
    };
    let status_change_usage_snapshot_exhausted = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_usage_snapshot_exhausted,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_usage_snapshot_exhausted,
    };
    let status_change_quota_still_exhausted = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_quota_still_exhausted,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_quota_still_exhausted,
    };
    let status_change_transport_failure = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_transport_failure,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_transport_failure,
    };
    let status_change_upstream_server_overloaded = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_server_overloaded,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_server_overloaded,
    };
    let status_change_upstream_http_5xx = match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX)
            .map_err(internal_error_tuple)?
        {
            OptionalField::Missing => row.policy_status_change_upstream_http_5xx,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_status_change_upstream_http_5xx,
    };
    let policy_image_first_byte_timeout_secs = match payload
        .routing_rule
        .as_ref()
        .and_then(|rule| rule.timeouts.as_ref())
    {
        Some(timeouts) => match timeouts.image_first_byte_timeout_secs {
            OptionalField::Missing => row.policy_image_first_byte_timeout_secs,
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                normalize_pool_routing_timeout_secs(Some(value), "imageFirstByteTimeoutSecs")?
                    .and_then(|value| i64::try_from(value).ok())
            }
        },
        None => row.policy_image_first_byte_timeout_secs,
    };
    let policy_available_models_mode = match payload.routing_rule.as_ref() {
        Some(rule) => match &rule.available_models_mode {
            OptionalField::Missing => match &rule.available_models {
                OptionalField::Value(_) => Some("allowlist".to_string()),
                OptionalField::Missing => row.policy_available_models_mode.clone(),
                OptionalField::Null => None,
            },
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                let normalized = value.trim().to_ascii_lowercase();
                if normalized != "allowlist" && normalized != "denylist" {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "availableModelsMode must be allowlist or denylist".to_string(),
                    ));
                }
                Some(normalized)
            }
        },
        None => row.policy_available_models_mode.clone(),
    };
    let policy_available_models_json = match payload.routing_rule.as_ref() {
        Some(rule) if matches!(rule.available_models_mode, OptionalField::Null) => None,
        Some(rule) => match &rule.available_models {
            OptionalField::Missing => row.policy_available_models_json.clone(),
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(
                encode_string_array_json(&normalize_available_models(
                    Some(value.clone()),
                    "availableModels",
                )?)
                .map_err(internal_error_tuple)?,
            ),
        },
        None => row.policy_available_models_json.clone(),
    };
    let now_iso = format_utc_iso(Utc::now());
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let effective_plan_type =
        resolve_effective_plan_type_for_account(tx.as_mut(), id, row.plan_type.as_deref())
            .await
            .map_err(internal_error_tuple)?;
    if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        ensure_display_name_available_for_oauth_identity(
            &mut *tx,
            &row.display_name,
            Some(id),
            row.chatgpt_account_id.as_deref(),
            row.chatgpt_user_id.as_deref(),
            row.group_name.as_deref(),
            effective_plan_type.as_deref(),
        )
        .await?;
    } else {
        ensure_display_name_available(&mut *tx, &row.display_name, Some(id)).await?;
    }
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            email = ?3,
            group_name = ?4,
            is_mother = ?5,
            note = ?6,
            enabled = ?7,
            masked_api_key = ?8,
            encrypted_credentials = ?9,
            local_primary_limit = ?10,
            local_secondary_limit = ?11,
            local_limit_unit = ?12,
            upstream_base_url = ?13,
            policy_allow_cut_out = ?14,
            policy_allow_cut_in = ?15,
            policy_priority_tier = ?16,
            policy_fast_mode_rewrite_mode = ?17,
            policy_image_tool_rewrite_mode = ?18,
            policy_request_compression_algorithm = ?19,
            policy_concurrency_limit = ?20,
            policy_upstream_429_retry_enabled = ?21,
            policy_upstream_429_max_retries = ?22,
            policy_available_models_json = ?23,
            policy_status_change_upstream_http_401 = ?24,
            policy_status_change_upstream_http_402 = ?25,
            policy_status_change_upstream_http_403 = ?26,
            policy_status_change_reauth_required = ?27,
            policy_status_change_upstream_http_429_rate_limit = ?28,
            policy_status_change_upstream_http_429_quota_exhausted = ?29,
            policy_status_change_usage_snapshot_exhausted = ?30,
            policy_status_change_quota_still_exhausted = ?31,
            policy_status_change_transport_failure = ?32,
            policy_status_change_upstream_server_overloaded = ?33,
            policy_status_change_upstream_http_5xx = ?34,
            policy_responses_first_byte_timeout_secs = ?35,
            policy_compact_first_byte_timeout_secs = ?36,
            policy_image_first_byte_timeout_secs = ?37,
            policy_responses_stream_timeout_secs = ?38,
            policy_compact_stream_timeout_secs = ?39,
            bound_proxy_keys_json = ?40,
            policy_response_endpoint_capability_override = ?41,
            policy_chat_completions_capability_override = ?42,
            policy_image_endpoint_capability_override = ?43,
            policy_response_image_tool_capability_override = ?44,
            policy_codex_imagegen_capability_override = ?45,
            policy_standalone_search_capability_override = ?46,
            updated_at = ?47,
            policy_codex_imagegen_rewrite_mode = ?48,
            policy_available_models_mode = ?49
        WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(&row.display_name)
    .bind(&row.email)
    .bind(&row.group_name)
    .bind(row.is_mother)
    .bind(&row.note)
    .bind(row.enabled)
    .bind(&row.masked_api_key)
    .bind(&row.encrypted_credentials)
    .bind(row.local_primary_limit)
    .bind(row.local_secondary_limit)
    .bind(&row.local_limit_unit)
    .bind(&row.upstream_base_url)
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.allow_cut_out {
            OptionalField::Missing => row.policy_allow_cut_out,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_allow_cut_out,
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.allow_cut_in {
            OptionalField::Missing => row.policy_allow_cut_in,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_allow_cut_in,
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.priority_tier {
            OptionalField::Missing => row.policy_priority_tier.clone(),
            OptionalField::Null => None,
            OptionalField::Value(_) => policy_priority_tier.clone(),
        },
        None => row.policy_priority_tier.clone(),
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.fast_mode_rewrite_mode {
            OptionalField::Missing => row.policy_fast_mode_rewrite_mode.clone(),
            OptionalField::Null => None,
            OptionalField::Value(_) => policy_fast_mode_rewrite_mode.clone(),
        },
        None => row.policy_fast_mode_rewrite_mode.clone(),
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.image_tool_rewrite_mode {
            OptionalField::Missing => row.policy_image_tool_rewrite_mode.clone(),
            OptionalField::Null => None,
            OptionalField::Value(_) => policy_image_tool_rewrite_mode.clone(),
        },
        None => row.policy_image_tool_rewrite_mode.clone(),
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.request_compression_algorithm {
            OptionalField::Missing => row.policy_request_compression_algorithm.clone(),
            OptionalField::Null => None,
            OptionalField::Value(_) => policy_request_compression_algorithm.clone(),
        },
        None => row.policy_request_compression_algorithm.clone(),
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.concurrency_limit {
            OptionalField::Missing => row.policy_concurrency_limit,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(normalize_concurrency_limit(
                Some(value),
                "concurrencyLimit",
            )?),
        },
        None => row.policy_concurrency_limit,
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.upstream_429_retry_enabled {
            OptionalField::Missing => row.policy_upstream_429_retry_enabled,
            OptionalField::Null => None,
            OptionalField::Value(value) => Some(if value { 1_i64 } else { 0_i64 }),
        },
        None => row.policy_upstream_429_retry_enabled,
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.upstream_429_max_retries {
            OptionalField::Missing => row.policy_upstream_429_max_retries,
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                Some(i64::from(normalize_group_upstream_429_max_retries(value)))
            }
        },
        None => row.policy_upstream_429_max_retries,
    })
    .bind(policy_available_models_json)
    .bind(status_change_upstream_http_401)
    .bind(status_change_upstream_http_402)
    .bind(status_change_upstream_http_403)
    .bind(status_change_reauth_required)
    .bind(status_change_upstream_http_429_rate_limit)
    .bind(status_change_upstream_http_429_quota_exhausted)
    .bind(status_change_usage_snapshot_exhausted)
    .bind(status_change_quota_still_exhausted)
    .bind(status_change_transport_failure)
    .bind(status_change_upstream_server_overloaded)
    .bind(status_change_upstream_http_5xx)
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .timeouts
            .clone()
            .unwrap_or_default()
            .responses_first_byte_timeout_secs
        {
            OptionalField::Missing => row.policy_responses_first_byte_timeout_secs,
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                normalize_pool_routing_timeout_secs(Some(value), "responsesFirstByteTimeoutSecs")?
                    .and_then(|value| i64::try_from(value).ok())
            }
        },
        None => row.policy_responses_first_byte_timeout_secs,
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .timeouts
            .clone()
            .unwrap_or_default()
            .compact_first_byte_timeout_secs
        {
            OptionalField::Missing => row.policy_compact_first_byte_timeout_secs,
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                normalize_pool_routing_timeout_secs(Some(value), "compactFirstByteTimeoutSecs")?
                    .and_then(|value| i64::try_from(value).ok())
            }
        },
        None => row.policy_compact_first_byte_timeout_secs,
    })
    .bind(policy_image_first_byte_timeout_secs)
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .timeouts
            .clone()
            .unwrap_or_default()
            .responses_stream_timeout_secs
        {
            OptionalField::Missing => row.policy_responses_stream_timeout_secs,
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                normalize_pool_routing_timeout_secs(Some(value), "responsesStreamTimeoutSecs")?
                    .and_then(|value| i64::try_from(value).ok())
            }
        },
        None => row.policy_responses_stream_timeout_secs,
    })
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule
            .timeouts
            .clone()
            .unwrap_or_default()
            .compact_stream_timeout_secs
        {
            OptionalField::Missing => row.policy_compact_stream_timeout_secs,
            OptionalField::Null => None,
            OptionalField::Value(value) => {
                normalize_pool_routing_timeout_secs(Some(value), "compactStreamTimeoutSecs")?
                    .and_then(|value| i64::try_from(value).ok())
            }
        },
        None => row.policy_compact_stream_timeout_secs,
    })
    .bind(next_bound_proxy_keys_json)
    .bind(response_endpoint_capability_override)
    .bind(chat_completions_capability_override)
    .bind(image_endpoint_capability_override)
    .bind(response_image_tool_capability_override)
    .bind(codex_imagegen_capability_override)
    .bind(standalone_search_capability_override)
    .bind(&now_iso)
    .bind(match payload.routing_rule.as_ref() {
        Some(rule) => match rule.codex_imagegen_rewrite_mode {
            OptionalField::Missing => row.policy_codex_imagegen_rewrite_mode.clone(),
            OptionalField::Null => None,
            OptionalField::Value(_) => policy_codex_imagegen_rewrite_mode.clone(),
        },
        None => row.policy_codex_imagegen_rewrite_mode.clone(),
    })
    .bind(policy_available_models_mode)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;
    if let Some(rule) = payload.routing_rule.as_ref()
        && !matches!(
            rule.oauth_request_compression_algorithm,
            OptionalField::Missing
        )
    {
        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_oauth_request_compression_algorithm = ?2 WHERE id = ?1",
        )
        .bind(id)
        .bind(policy_oauth_request_compression_algorithm)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    }
    if invalidate_runtime_outcomes {
        sqlx::query("UPDATE pool_upstream_accounts SET runtime_routing_generation=runtime_routing_generation+1 WHERE id=?1")
            .bind(id).execute(tx.as_mut()).await.map_err(internal_error_tuple)?;
    }
    if !is_api_key_account {
        apply_mother_assignment(&mut tx, id, row.group_name.as_deref(), row.is_mother != 0)
            .await
            .map_err(internal_error_tuple)?;
        save_group_metadata_after_account_write(
            tx.as_mut(),
            row.group_name.as_deref(),
            &requested_group_metadata_changes,
            previous_group_name == row.group_name,
        )
        .await
        .map_err(internal_error_tuple)?;
        if previous_group_name != row.group_name {
            cleanup_orphaned_group_metadata(tx.as_mut(), previous_group_name.as_deref())
                .await
                .map_err(internal_error_tuple)?;
        }
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    if clear_hard_failure_after_update {
        set_account_status(&state.pool, id, UPSTREAM_ACCOUNT_STATUS_ACTIVE, None)
            .await
            .map_err(internal_error_tuple)?;
    }
    record_account_update_action(&state.pool, id, "account settings were updated")
        .await
        .map_err(internal_error_tuple)?;
    let routing_scope = if previous_group_name != row.group_name
        || requested_group_metadata_changes.was_requested()
    {
        None
    } else {
        Some(std::slice::from_ref(&id))
    };
    let routing_state_version =
        match publish_account_effective_routing_rules_changed(state, routing_scope, &[]).await {
            Ok(version) => Some(version),
            Err(err) => {
                warn!(
                    ?err,
                    account_id = id,
                    "account update committed but routing publication failed"
                );
                invalidate_dashboard_activity_snapshots_with_accounts(
                    state.dashboard_activity_snapshot_cache.as_ref(),
                    "account_effective_routing_rules_publication_failed",
                )
                .await;
                None
            }
        };
    let refreshed_row = load_upstream_account_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    if !was_fresh_routable
        && is_account_selectable_for_fresh_assignment(&refreshed_row, false, Utc::now())
    {
        publish_pool_routing_availability(state);
    }

    let mut detail = load_upstream_account_detail_with_actual_usage(state, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    detail.routing_state_version = routing_state_version;
    Ok(detail)
}
