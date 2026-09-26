use super::*;

pub(crate) async fn update_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
    Json(payload): Json<UpdateUpstreamAccountGroupRequest>,
) -> Result<Json<UpstreamAccountGroupSummary>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    let group_name = normalize_optional_text(Some(group_name)).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "group name is required".to_string(),
        )
    })?;
    let note = normalize_optional_text(payload.note);
    let bound_proxy_keys_was_updated = payload.bound_proxy_keys.is_some();
    let mut bound_proxy_keys = payload
        .bound_proxy_keys
        .map(normalize_bound_proxy_keys)
        .unwrap_or_else(Vec::new);
    let node_shunt_enabled_was_updated = payload.node_shunt_enabled.is_some();
    let single_account_rotation_enabled_was_updated =
        payload.single_account_rotation_enabled.is_some();
    let upstream_429_retry_enabled_was_updated = payload.upstream_429_retry_enabled.is_some();
    let upstream_429_max_retries_was_updated = payload.upstream_429_max_retries.is_some();
    let normalized_upstream_429_max_retries = payload
        .upstream_429_max_retries
        .map(normalize_group_upstream_429_max_retries)
        .unwrap_or_default();
    let concurrency_limit =
        normalize_concurrency_limit(payload.concurrency_limit.or(Some(0)), "concurrencyLimit")?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let existing_metadata = load_group_metadata_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?
        .unwrap_or_default();
    if bound_proxy_keys_was_updated {
        bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state.as_ref(), &bound_proxy_keys)
            .await
            .map_err(internal_error_tuple)?;
    }
    let next_bound_proxy_keys = if bound_proxy_keys_was_updated {
        bound_proxy_keys
    } else {
        existing_metadata.bound_proxy_keys.clone()
    };
    let next_node_shunt_enabled = if node_shunt_enabled_was_updated {
        payload.node_shunt_enabled.unwrap_or(false)
    } else {
        existing_metadata.node_shunt_enabled
    };
    let node_shunt_was_disabled = existing_metadata.node_shunt_enabled
        && node_shunt_enabled_was_updated
        && !next_node_shunt_enabled;
    if next_node_shunt_enabled && next_bound_proxy_keys.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            missing_group_bound_proxy_error_message(group_name.trim()),
        ));
    }
    if !next_node_shunt_enabled
        && !next_bound_proxy_keys.is_empty()
        && (bound_proxy_keys_was_updated || node_shunt_was_disabled)
    {
        let has_selectable_bound_proxy_keys = {
            let manager = state.forward_proxy.lock().await;
            manager.has_selectable_bound_proxy_keys(&next_bound_proxy_keys)
        };
        if !has_selectable_bound_proxy_keys {
            return Err((
                StatusCode::BAD_REQUEST,
                "select at least one available proxy node or clear bindings before saving"
                    .to_string(),
            ));
        }
    }
    save_group_metadata_record_conn(
        tx.as_mut(),
        &group_name,
        UpstreamAccountGroupMetadata {
            note,
            bound_proxy_keys: next_bound_proxy_keys,
            node_shunt_enabled: next_node_shunt_enabled,
            single_account_rotation_enabled: if single_account_rotation_enabled_was_updated {
                payload.single_account_rotation_enabled.unwrap_or(false)
            } else {
                existing_metadata.single_account_rotation_enabled
            },
            upstream_429_retry_enabled: if upstream_429_retry_enabled_was_updated {
                payload.upstream_429_retry_enabled.unwrap_or(false)
            } else {
                existing_metadata.upstream_429_retry_enabled
            },
            upstream_429_max_retries: if upstream_429_max_retries_was_updated {
                normalized_upstream_429_max_retries
            } else {
                existing_metadata.upstream_429_max_retries
            },
            concurrency_limit: payload
                .concurrency_limit
                .map(|_| concurrency_limit)
                .unwrap_or(existing_metadata.concurrency_limit),
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    if let Some(routing_rule) = payload.routing_rule.as_ref() {
        let policy_concurrency_limit = match routing_rule.concurrency_limit {
            OptionalField::Value(value) => Some(normalize_concurrency_limit(
                Some(value),
                "concurrencyLimit",
            )?),
            OptionalField::Missing | OptionalField::Null => None,
        };
        let policy_priority_tier = routing_rule
            .priority_tier_value()
            .map(|value| normalize_tag_priority_tier(Some(value)).map(|tier| tier.as_str()))
            .transpose()?;
        let policy_fast_mode_rewrite_mode = routing_rule
            .fast_mode_rewrite_mode_value()
            .map(|value| {
                normalize_tag_fast_mode_rewrite_mode(Some(value)).map(|mode| mode.as_str())
            })
            .transpose()?;
        let policy_image_tool_rewrite_mode = routing_rule
            .image_tool_rewrite_mode_value()
            .map(|value| {
                super::sync::normalize_upstream_image_tool_rewrite_mode(Some(value))
                    .map(|mode| mode.as_str())
            })
            .transpose()?;
        let policy_codex_imagegen_rewrite_mode = routing_rule
            .codex_imagegen_rewrite_mode_value()
            .map(|value| {
                super::sync::normalize_codex_imagegen_rewrite_mode(Some(value))
                    .map(|mode| mode.as_str())
            })
            .transpose()?;
        let policy_request_compression_algorithm = routing_rule
            .request_compression_algorithm_value()
            .map(|value| {
                normalize_request_compression_algorithm(Some(value)).map(|mode| mode.as_str())
            })
            .transpose()?;
        let mut available_models_json = match &routing_rule.available_models {
            OptionalField::Missing | OptionalField::Null => None,
            OptionalField::Value(value) => Some(
                encode_string_array_json(&normalize_available_models(
                    Some(value.clone()),
                    "availableModels",
                )?)
                .map_err(internal_error_tuple)?,
            ),
        };
        let available_models_mode = match &routing_rule.available_models_mode {
            OptionalField::Missing => match &routing_rule.available_models {
                OptionalField::Value(_) => Some("allowlist".to_string()),
                OptionalField::Missing | OptionalField::Null => None,
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
        };
        if matches!(routing_rule.available_models_mode, OptionalField::Null) {
            available_models_json = None;
        }
        // A mode-only patch changes interpretation, not the selected IDs.
        // Preserve the stored JSON unless the caller explicitly clears mode
        // or submits a replacement list.
        let preserve_available_models_json =
            matches!(routing_rule.available_models, OptionalField::Missing)
                && !matches!(routing_rule.available_models_mode, OptionalField::Null);
        let timeout_patch = routing_rule.timeouts.clone().unwrap_or_default();
        let responses_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.responses_first_byte_timeout_secs,
            "responsesFirstByteTimeoutSecs",
        )?;
        let compact_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.compact_first_byte_timeout_secs,
            "compactFirstByteTimeoutSecs",
        )?;
        let image_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.image_first_byte_timeout_secs,
            "imageFirstByteTimeoutSecs",
        )?;
        let responses_stream_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.responses_stream_timeout_secs,
            "responsesStreamTimeoutSecs",
        )?;
        let compact_stream_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.compact_stream_timeout_secs,
            "compactStreamTimeoutSecs",
        )?;
        let status_change_upstream_http_401 = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_402 = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_403 = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403)
            .map_err(internal_error_tuple)?;
        let status_change_reauth_required = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_429_rate_limit = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_429_quota_exhausted = routing_rule
            .status_change_reason_field(
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            )
            .map_err(internal_error_tuple)?;
        let status_change_usage_snapshot_exhausted = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
            .map_err(internal_error_tuple)?;
        let status_change_quota_still_exhausted = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
            .map_err(internal_error_tuple)?;
        let status_change_transport_failure = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_server_overloaded = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_5xx = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX)
            .map_err(internal_error_tuple)?;
        sqlx::query(
            r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_allow_cut_out = CASE WHEN ?2 != 0 THEN policy_allow_cut_out ELSE ?3 END,
                policy_allow_cut_in = CASE WHEN ?4 != 0 THEN policy_allow_cut_in ELSE ?5 END,
                policy_priority_tier = CASE WHEN ?6 != 0 THEN policy_priority_tier ELSE ?7 END,
                policy_fast_mode_rewrite_mode = CASE WHEN ?8 != 0 THEN policy_fast_mode_rewrite_mode ELSE ?9 END,
                policy_image_tool_rewrite_mode = CASE WHEN ?10 != 0 THEN policy_image_tool_rewrite_mode ELSE ?11 END,
                policy_request_compression_algorithm = CASE WHEN ?12 != 0 THEN policy_request_compression_algorithm ELSE ?13 END,
                policy_concurrency_limit = CASE WHEN ?14 != 0 THEN policy_concurrency_limit ELSE ?15 END,
                policy_upstream_429_retry_enabled = CASE WHEN ?16 != 0 THEN policy_upstream_429_retry_enabled ELSE ?17 END,
                policy_upstream_429_max_retries = CASE WHEN ?18 != 0 THEN policy_upstream_429_max_retries ELSE ?19 END,
                policy_available_models_json = CASE
                    WHEN ?20 != 0 THEN policy_available_models_json
                    ELSE ?21
                END,
                policy_status_change_upstream_http_401 = CASE WHEN ?22 != 0 THEN policy_status_change_upstream_http_401 ELSE ?23 END,
                policy_status_change_upstream_http_402 = CASE WHEN ?24 != 0 THEN policy_status_change_upstream_http_402 ELSE ?25 END,
                policy_status_change_upstream_http_403 = CASE WHEN ?26 != 0 THEN policy_status_change_upstream_http_403 ELSE ?27 END,
                policy_status_change_reauth_required = CASE WHEN ?28 != 0 THEN policy_status_change_reauth_required ELSE ?29 END,
                policy_status_change_upstream_http_429_rate_limit = CASE WHEN ?30 != 0 THEN policy_status_change_upstream_http_429_rate_limit ELSE ?31 END,
                policy_status_change_upstream_http_429_quota_exhausted = CASE WHEN ?32 != 0 THEN policy_status_change_upstream_http_429_quota_exhausted ELSE ?33 END,
                policy_status_change_usage_snapshot_exhausted = CASE WHEN ?34 != 0 THEN policy_status_change_usage_snapshot_exhausted ELSE ?35 END,
                policy_status_change_quota_still_exhausted = CASE WHEN ?36 != 0 THEN policy_status_change_quota_still_exhausted ELSE ?37 END,
                policy_status_change_transport_failure = CASE WHEN ?38 != 0 THEN policy_status_change_transport_failure ELSE ?39 END,
                policy_status_change_upstream_server_overloaded = CASE WHEN ?40 != 0 THEN policy_status_change_upstream_server_overloaded ELSE ?41 END,
                policy_status_change_upstream_http_5xx = CASE WHEN ?42 != 0 THEN policy_status_change_upstream_http_5xx ELSE ?43 END,
                policy_responses_first_byte_timeout_secs = CASE WHEN ?44 != 0 THEN policy_responses_first_byte_timeout_secs ELSE ?45 END,
                policy_compact_first_byte_timeout_secs = CASE WHEN ?46 != 0 THEN policy_compact_first_byte_timeout_secs ELSE ?47 END,
                policy_image_first_byte_timeout_secs = CASE WHEN ?48 != 0 THEN policy_image_first_byte_timeout_secs ELSE ?49 END,
                policy_responses_stream_timeout_secs = CASE WHEN ?50 != 0 THEN policy_responses_stream_timeout_secs ELSE ?51 END,
                policy_compact_stream_timeout_secs = CASE WHEN ?52 != 0 THEN policy_compact_stream_timeout_secs ELSE ?53 END,
                policy_codex_imagegen_rewrite_mode = CASE WHEN ?54 != 0 THEN policy_codex_imagegen_rewrite_mode ELSE ?55 END
            WHERE group_name = ?1
            "#,
        )
        .bind(&group_name)
        .bind(if matches!(routing_rule.allow_cut_out, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&routing_rule.allow_cut_out))
        .bind(if matches!(routing_rule.allow_cut_in, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&routing_rule.allow_cut_in))
        .bind(if matches!(routing_rule.priority_tier, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_priority_tier)
        .bind(if matches!(routing_rule.fast_mode_rewrite_mode, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_fast_mode_rewrite_mode)
        .bind(if matches!(routing_rule.image_tool_rewrite_mode, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_image_tool_rewrite_mode)
        .bind(if matches!(routing_rule.request_compression_algorithm, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_request_compression_algorithm)
        .bind(if matches!(routing_rule.concurrency_limit, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_concurrency_limit)
        .bind(if matches!(routing_rule.upstream_429_retry_enabled, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&routing_rule.upstream_429_retry_enabled))
        .bind(if matches!(routing_rule.upstream_429_max_retries, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_retry_count_to_i64(&routing_rule.upstream_429_max_retries))
        .bind(if preserve_available_models_json { 1_i64 } else { 0_i64 })
        .bind(available_models_json)
        .bind(if matches!(status_change_upstream_http_401, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_401))
        .bind(if matches!(status_change_upstream_http_402, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_402))
        .bind(if matches!(status_change_upstream_http_403, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_403))
        .bind(if matches!(status_change_reauth_required, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_reauth_required))
        .bind(if matches!(status_change_upstream_http_429_rate_limit, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_429_rate_limit))
        .bind(if matches!(status_change_upstream_http_429_quota_exhausted, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_429_quota_exhausted))
        .bind(if matches!(status_change_usage_snapshot_exhausted, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_usage_snapshot_exhausted))
        .bind(if matches!(status_change_quota_still_exhausted, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_quota_still_exhausted))
        .bind(if matches!(status_change_transport_failure, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_transport_failure))
        .bind(if matches!(status_change_upstream_server_overloaded, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_server_overloaded))
        .bind(if matches!(status_change_upstream_http_5xx, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_5xx))
        .bind(if responses_first_byte_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(responses_first_byte_timeout_secs.flatten())
        .bind(if compact_first_byte_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(compact_first_byte_timeout_secs.flatten())
        .bind(if image_first_byte_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(image_first_byte_timeout_secs.flatten())
        .bind(if responses_stream_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(responses_stream_timeout_secs.flatten())
        .bind(if compact_stream_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(compact_stream_timeout_secs.flatten())
        .bind(if matches!(routing_rule.codex_imagegen_rewrite_mode, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_codex_imagegen_rewrite_mode)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        if !matches!(routing_rule.available_models_mode, OptionalField::Missing)
            || matches!(
                routing_rule.available_models,
                OptionalField::Value(_) | OptionalField::Null
            )
        {
            sqlx::query(
                "UPDATE pool_upstream_account_group_notes SET policy_available_models_mode = ?2 WHERE group_name = ?1",
            )
            .bind(&group_name)
            .bind(available_models_mode)
            .execute(tx.as_mut())
            .await
            .map_err(internal_error_tuple)?;
        }
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Err(err) =
        publish_account_effective_routing_rules_changed(state.as_ref(), None, &[]).await
    {
        warn!(?err, group_name = %group_name, "group update committed but routing publication failed");
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }

    let saved = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    let mut conn = state.pool.acquire().await.map_err(internal_error_tuple)?;
    let account_count = group_account_count_conn(&mut conn, &group_name)
        .await
        .map_err(internal_error_tuple)?;
    let routing_rule = load_group_routing_rule(&state.pool, &group_name)
        .await
        .map_err(internal_error_tuple)?
        .clone();
    let (effective_timeouts, timeout_field_sources, _) =
        load_effective_request_path_timeouts_for_group(
            &state.pool,
            &state.config,
            Some(&group_name),
        )
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountGroupSummary {
        group_name: group_name.clone(),
        account_count,
        note: saved.note,
        bound_proxy_keys: saved.bound_proxy_keys,
        node_shunt_enabled: saved.node_shunt_enabled,
        single_account_rotation_enabled: saved.single_account_rotation_enabled,
        upstream_429_retry_enabled: saved.upstream_429_retry_enabled,
        upstream_429_max_retries: saved.upstream_429_max_retries,
        concurrency_limit: saved.concurrency_limit,
        routing_rule,
        effective_timeouts,
        timeout_field_sources,
    }))
}

pub(crate) async fn delete_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    let group_name = normalize_optional_text(Some(group_name)).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "group name is required".to_string(),
        )
    })?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let account_count = group_account_count_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?;
    if account_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "group still has {account_count} account{}; move them out before deleting",
                if account_count == 1 { "" } else { "s" }
            ),
        ));
    }
    let deleted = sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(&group_name)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?
    .rows_affected();
    if deleted == 0 {
        return Err((StatusCode::NOT_FOUND, "group not found".to_string()));
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}
