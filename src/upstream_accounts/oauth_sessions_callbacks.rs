use super::*;

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
    let crypto_key = if api_key.is_some() {
        Some(state.upstream_accounts.require_crypto_key()?)
    } else {
        None
    };
    if api_key.is_some() || timeout_updates.is_some() {
        save_pool_routing_settings(
            &state.pool,
            &state.config,
            crypto_key,
            api_key.as_deref(),
            timeout_updates.as_ref(),
        )
        .await?;
        if api_key.is_some() {
            refresh_pool_routing_runtime_cache(state.as_ref())
                .await
                .map_err(internal_error_tuple)?;
        } else {
            refresh_pool_routing_runtime_cache_best_effort(
                state.as_ref(),
                "timeout-only settings update",
            )
            .await;
        }
    }
    if payload.maintenance.is_some() {
        save_pool_routing_maintenance_settings(&state.pool, merged_maintenance)
            .await
            .map_err(internal_error_tuple)?;
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

pub(crate) async fn create_oauth_mailbox_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOauthMailboxSessionRequest>,
) -> Result<Json<OauthMailboxSessionResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    cleanup_expired_oauth_mailbox_sessions(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let config = upstream_mailbox_config(&state.config)?;
    if let Some(manual_email_address) =
        match requested_manual_mailbox_address(payload.email_address.as_deref()) {
            RequestedManualMailboxAddress::Missing => None,
            RequestedManualMailboxAddress::Valid(value) => Some(value),
            RequestedManualMailboxAddress::Invalid(invalid_email_address) => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    invalid_email_address,
                    "invalid_format",
                )));
            }
        }
    {
        if !mailbox_address_is_valid(&manual_email_address) {
            return Ok(Json(oauth_mailbox_session_unsupported_response(
                manual_email_address,
                "invalid_format",
            )));
        }
        let moemail_config = moemail_get_config(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?;
        let supported_domains = moemail_supported_domains(&moemail_config);
        let email_domain = manual_email_address
            .split('@')
            .nth(1)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !supported_domains.is_empty() && !supported_domains.contains(&email_domain) {
            return Ok(Json(oauth_mailbox_session_unsupported_response(
                manual_email_address,
                "unsupported_domain",
            )));
        }
        let existing_remote_mailbox = moemail_list_emails(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?
            .into_iter()
            .find(|item| {
                normalize_mailbox_address(&item.address) == Some(manual_email_address.clone())
            });
        let Some(remote_mailbox) = existing_remote_mailbox else {
            let generated = moemail_create_email_for_address(
                &state.http_clients.shared,
                config,
                &manual_email_address,
            )
            .await
            .map_err(internal_error_tuple)?;
            let email_address = generated.email.trim().to_string();
            let email_domain = email_address
                .split('@')
                .nth(1)
                .unwrap_or(config.default_domain.as_str())
                .to_string();
            let session_id = random_hex(16)?;
            let now = Utc::now();
            let expires_at = now
                + ChronoDuration::seconds(
                    DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64,
                );
            let now_iso = format_utc_iso(now);
            let expires_at_iso = format_utc_iso(expires_at);
            sqlx::query(
                r#"
                INSERT INTO pool_oauth_mailbox_sessions (
                    session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
                    latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
                    invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
                    expires_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, NULL, ?6, ?6, ?7)
                "#,
            )
            .bind(&session_id)
            .bind(&generated.id)
            .bind(&email_address)
            .bind(&email_domain)
            .bind(OAUTH_MAILBOX_SOURCE_GENERATED)
            .bind(&now_iso)
            .bind(&expires_at_iso)
            .execute(&state.pool)
            .await
            .map_err(internal_error_tuple)?;

            return Ok(Json(oauth_mailbox_session_supported_response(
                session_id,
                email_address,
                expires_at_iso,
                OAUTH_MAILBOX_SOURCE_GENERATED,
            )));
        };
        let mut remote_messages = match moemail_list_messages_for_attach(
            &state.http_clients.shared,
            config,
            &remote_mailbox.id,
        )
        .await
        .map_err(internal_error_tuple)?
        {
            MoeMailAttachReadState::Readable(messages) => messages,
            MoeMailAttachReadState::NotReadable => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    manual_email_address,
                    "not_readable",
                )));
            }
        };
        sort_mailbox_messages_desc(&mut remote_messages);
        let latest_message_id = latest_mailbox_message_id(&remote_messages);
        let (latest_code, latest_invite) = match resolve_mailbox_message_state_for_attach(
            &state.http_clients.shared,
            config,
            &remote_mailbox.id,
            &remote_messages,
        )
        .await
        .map_err(internal_error_tuple)?
        {
            MoeMailAttachReadState::Readable(state) => state,
            MoeMailAttachReadState::NotReadable => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    manual_email_address,
                    "not_readable",
                )));
            }
        };
        let session_id = random_hex(16)?;
        let now = Utc::now();
        let expires_at = normalize_mailbox_session_expires_at(
            remote_mailbox.expires_at.as_deref(),
            now + ChronoDuration::seconds(
                DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64,
            ),
        );
        let now_iso = format_utc_iso(now);
        sqlx::query(
            r#"
            INSERT INTO pool_oauth_mailbox_sessions (
                session_id, remote_email_id, email_address, email_domain, mailbox_source,
                latest_code_value, latest_code_source, latest_code_updated_at, invite_subject,
                invite_copy_value, invite_copy_label, invite_updated_at, invited, last_message_id,
                created_at, updated_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15, ?16)
            "#,
        )
        .bind(&session_id)
        .bind(&remote_mailbox.id)
        .bind(&manual_email_address)
        .bind(&email_domain)
        .bind(OAUTH_MAILBOX_SOURCE_ATTACHED)
        .bind(latest_code.as_ref().map(|value| value.value.clone()))
        .bind(latest_code.as_ref().map(|value| value.source.clone()))
        .bind(latest_code.as_ref().map(|value| value.updated_at.clone()))
        .bind(latest_invite.as_ref().map(|value| value.subject.clone()))
        .bind(latest_invite.as_ref().map(|value| value.copy_value.clone()))
        .bind(latest_invite.as_ref().map(|value| value.copy_label.clone()))
        .bind(latest_invite.as_ref().map(|value| value.updated_at.clone()))
        .bind(if latest_invite.is_some() { 1 } else { 0 })
        .bind(latest_message_id)
        .bind(&now_iso)
        .bind(&expires_at)
        .execute(&state.pool)
        .await
        .map_err(internal_error_tuple)?;

        return Ok(Json(oauth_mailbox_session_supported_response(
            session_id,
            manual_email_address,
            expires_at,
            OAUTH_MAILBOX_SOURCE_ATTACHED,
        )));
    }
    let generated = moemail_create_email(&state.http_clients.shared, config)
        .await
        .map_err(internal_error_tuple)?;
    let email_address = generated.email.trim().to_string();
    let email_domain = email_address
        .split('@')
        .nth(1)
        .unwrap_or(config.default_domain.as_str())
        .to_string();
    let session_id = random_hex(16)?;
    let now = Utc::now();
    let expires_at =
        now + ChronoDuration::seconds(DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64);
    let now_iso = format_utc_iso(now);
    let expires_at_iso = format_utc_iso(expires_at);
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_mailbox_sessions (
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 0, NULL, ?6, ?6, ?7)
        "#,
    )
    .bind(&session_id)
    .bind(&generated.id)
    .bind(&email_address)
    .bind(&email_domain)
    .bind(OAUTH_MAILBOX_SOURCE_GENERATED)
    .bind(&now_iso)
    .bind(&expires_at_iso)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple)?;

    Ok(Json(oauth_mailbox_session_supported_response(
        session_id,
        email_address,
        expires_at_iso,
        OAUTH_MAILBOX_SOURCE_GENERATED,
    )))
}

pub(crate) async fn get_oauth_mailbox_session_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<OauthMailboxStatusRequest>,
) -> Result<Json<OauthMailboxStatusBatchResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    cleanup_expired_oauth_mailbox_sessions(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let session_ids = payload
        .session_ids
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let rows = load_oauth_mailbox_sessions(&state.pool, &session_ids)
        .await
        .map_err(internal_error_tuple)?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        match refresh_oauth_mailbox_session_status(state.as_ref(), &row).await {
            Ok(refreshed) => items.push(oauth_mailbox_status_from_row(&refreshed)),
            Err(error) => {
                let mut status = oauth_mailbox_status_from_row(&row);
                status.error = Some(error.to_string());
                items.push(status);
            }
        }
    }
    Ok(Json(OauthMailboxStatusBatchResponse { items }))
}

pub(crate) async fn delete_oauth_mailbox_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(session_id): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let Some(row) = load_oauth_mailbox_session(&state.pool, &session_id)
        .await
        .map_err(internal_error_tuple)?
    else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if row.mailbox_source.as_deref() != Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
        && let Some(config) = state.config.upstream_accounts_moemail.as_ref()
        && let Err(err) =
            moemail_delete_email(&state.http_clients.shared, config, &row.remote_email_id).await
    {
        debug!(
            mailbox_session_id = %row.session_id,
            remote_email_id = %row.remote_email_id,
            error = %err,
            "failed to delete moemail mailbox during explicit cleanup"
        );
    }
    delete_oauth_mailbox_session_with_executor(&state.pool, &session_id)
        .await
        .map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn create_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOauthLoginSessionRequest>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    validate_mailbox_binding(
        &state.pool,
        payload.mailbox_session_id.as_deref(),
        payload.mailbox_address.as_deref(),
    )
    .await?;
    let tag_ids = validate_tag_ids(&state.pool, &payload.tag_ids).await?;
    let tag_ids_json = encode_tag_ids_json(&tag_ids).map_err(internal_error_tuple)?;

    let mut preserved_mother_flag = false;
    let mut preserved_display_name = None;
    let mut preserved_group_name = None;
    let mut preserved_note = None;
    let mut preserved_group_concurrency_limit = 0;

    if let Some(account_id) = payload.account_id {
        let Some(existing) = load_upstream_account_row(&state.pool, account_id)
            .await
            .map_err(internal_error_tuple)?
        else {
            return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
        };
        if existing.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
            return Err((
                StatusCode::BAD_REQUEST,
                "only OAuth accounts can be re-linked".to_string(),
            ));
        }
        preserved_mother_flag = existing.is_mother != 0;
        preserved_display_name = Some(existing.display_name);
        preserved_group_name = existing.group_name;
        preserved_note = existing.note;
        preserved_group_concurrency_limit =
            load_group_metadata(&state.pool, preserved_group_name.as_deref())
                .await
                .map_err(internal_error_tuple)?
                .concurrency_limit;
    }

    let is_mother = payload.is_mother.unwrap_or(preserved_mother_flag);
    let display_name = normalize_optional_text(payload.display_name).or(preserved_display_name);
    let group_name = normalize_optional_text(payload.group_name).or(preserved_group_name);
    let note = normalize_optional_text(payload.note).or(preserved_note);
    let requested_group_concurrency_limit =
        normalize_concurrency_limit(payload.concurrency_limit, "concurrencyLimit")?;
    let group_concurrency_limit = payload
        .concurrency_limit
        .map(|_| requested_group_concurrency_limit)
        .unwrap_or(preserved_group_concurrency_limit);
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        payload.group_bound_proxy_keys.clone(),
        payload.group_node_shunt_enabled,
    )
    .await?;

    let redirect_uri = build_manual_callback_redirect_uri().map_err(internal_error_tuple)?;
    let login_id = random_hex(16)?;
    let state_token = random_hex(32)?;
    let pkce_verifier = random_hex(64)?;
    let code_challenge = code_challenge_for_verifier(&pkce_verifier);
    let auth_url = build_oauth_authorize_url(
        &state.config.upstream_accounts_oauth_issuer,
        &state.config.upstream_accounts_oauth_client_id,
        &redirect_uri,
        &state_token,
        &code_challenge,
    )
    .map_err(internal_error_tuple)?;
    let now = Utc::now();
    let expires_at = now
        + ChronoDuration::seconds(state.config.upstream_accounts_login_session_ttl.as_secs() as i64);
    let now_iso = format_utc_iso(now);
    let expires_at_iso = format_utc_iso(expires_at);
    let group_note = normalize_optional_text(payload.group_note.clone());
    validate_group_note_target(group_name.as_deref(), payload.group_note.is_some())?;
    let store_group_note = if payload.group_note.is_some() {
        if let Some(group_name) = group_name.as_deref() {
            !group_has_accounts(&state.pool, group_name)
                .await
                .map_err(internal_error_tuple)?
        } else {
            false
        }
    } else {
        false
    };
    let stored_group_note = if store_group_note { group_note } else { None };

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    if let Some(display_name) = display_name.as_deref() {
        ensure_display_name_available(&mut *tx, display_name, payload.account_id).await?;
    }

    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, group_name, group_bound_proxy_keys_json, group_node_shunt_enabled,
            group_node_shunt_enabled_requested, is_mother, note, tag_ids_json, group_note, group_concurrency_limit,
            mailbox_session_id, generated_mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, expires_at, consumed_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, NULL, ?20, NULL, ?21, ?21)
        "#,
    )
    .bind(&login_id)
    .bind(payload.account_id)
    .bind(display_name)
    .bind(&resolved_group_binding.group_name)
    .bind(
        encode_group_bound_proxy_keys_json(&resolved_group_binding.bound_proxy_keys)
            .map_err(internal_error_tuple)?,
    )
    .bind(if resolved_group_binding.node_shunt_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(if payload.group_node_shunt_enabled.is_some() {
        1_i64
    } else {
        0_i64
    })
    .bind(if is_mother { 1 } else { 0 })
    .bind(note)
    .bind(tag_ids_json)
    .bind(stored_group_note)
    .bind(group_concurrency_limit)
    .bind(normalize_optional_text(payload.mailbox_session_id.clone()))
    .bind(normalize_optional_text(payload.mailbox_address.clone()))
    .bind(&state_token)
    .bind(&pkce_verifier)
    .bind(&redirect_uri)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .bind(&auth_url)
    .bind(&expires_at_iso)
    .bind(&now_iso)
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;

    Ok(Json(LoginSessionStatusResponse {
        login_id,
        status: LOGIN_SESSION_STATUS_PENDING.to_string(),
        auth_url: Some(auth_url),
        redirect_uri: Some(redirect_uri),
        expires_at: expires_at_iso,
        updated_at: now_iso,
        account_id: payload.account_id,
        error: None,
        sync_applied: None,
    }))
}

pub(crate) async fn get_oauth_login_session(
    State(state): State<Arc<AppState>>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id(&state.pool, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    Ok(Json(login_session_to_response(&session)))
}

pub(crate) async fn update_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(login_id): AxumPath<String>,
    Json(payload): Json<UpdateOauthLoginSessionRequest>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id_with_executor(&mut *tx, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    let requested_base_updated_at = headers
        .get(LOGIN_SESSION_BASE_UPDATED_AT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let completed_race_repair_requested = session.status == LOGIN_SESSION_STATUS_COMPLETED
        && session.account_id.is_some()
        && session
            .consumed_at
            .as_deref()
            .is_some_and(|value| value != session.updated_at)
        && requested_base_updated_at
            .as_deref()
            .is_some_and(|value| value == session.updated_at);
    // Completed-session repairs are only valid for create-account sessions that
    // still preserve their last pending baseline after callback completion.
    // Relogin sessions advance updated_at when they complete, so they never
    // qualify for this narrow repair path.
    let allows_completed_race_repair = if completed_race_repair_requested {
        let account_id = session.account_id.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "completed session account is missing".to_string(),
            )
        })?;
        let account = load_upstream_account_row_conn(tx.as_mut(), account_id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
        let current_group_metadata = match session.group_name.as_deref() {
            Some(group_name) => load_group_metadata_conn(tx.as_mut(), group_name)
                .await
                .map_err(internal_error_tuple)?
                .unwrap_or_default(),
            None => UpstreamAccountGroupMetadata::default(),
        };
        let current_tag_ids = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
        )
        .bind(account_id)
        .fetch_all(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        let session_group_node_shunt_enabled_requested =
            decode_group_requested_flag(session.group_node_shunt_enabled_requested);
        session.display_name.as_deref() == Some(account.display_name.as_str())
            && session.group_name == account.group_name
            && session.note == account.note
            && session.group_note == current_group_metadata.note
            && decode_group_bound_proxy_keys_json(session.group_bound_proxy_keys_json.as_deref())
                == current_group_metadata.bound_proxy_keys
            && session.group_concurrency_limit == current_group_metadata.concurrency_limit
            && (!session_group_node_shunt_enabled_requested
                || decode_group_node_shunt_enabled(session.group_node_shunt_enabled)
                    == current_group_metadata.node_shunt_enabled)
            && session.group_concurrency_limit == current_group_metadata.concurrency_limit
            && (session.is_mother != 0) == (account.is_mother != 0)
            && parse_tag_ids_json(session.tag_ids_json.as_deref()) == current_tag_ids
    } else {
        false
    };
    if session.status != LOGIN_SESSION_STATUS_PENDING && !allows_completed_race_repair {
        return Err((
            StatusCode::BAD_REQUEST,
            if session.status == LOGIN_SESSION_STATUS_EXPIRED {
                "The login session has expired. Please create a new authorization link.".to_string()
            } else {
                "This login session can no longer be edited.".to_string()
            },
        ));
    }
    if session.account_id.is_some() && session.status == LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session belongs to an existing account and cannot be edited.".to_string(),
        ));
    }
    if session.status == LOGIN_SESSION_STATUS_PENDING {
        if let Some(requested_base_updated_at) = requested_base_updated_at.as_deref() {
            if requested_base_updated_at != session.updated_at {
                tx.commit().await.map_err(internal_error_tuple)?;
                return Ok(Json(login_session_to_response_with_sync_applied(
                    &session, false,
                )));
            }
        }
    }

    let UpdateOauthLoginSessionRequest {
        display_name: requested_display_name,
        group_name: requested_group_name,
        group_bound_proxy_keys: requested_group_bound_proxy_keys,
        group_node_shunt_enabled: requested_group_node_shunt_enabled,
        note: requested_note,
        group_note: requested_group_note,
        concurrency_limit: requested_concurrency_limit,
        tag_ids: requested_tag_ids,
        is_mother: requested_is_mother,
        mailbox_session_id: requested_mailbox_session_id,
        mailbox_address: requested_mailbox_address,
    } = payload;
    let requested_group_name_was_updated = !matches!(requested_group_name, OptionalField::Missing);
    let requested_group_bound_proxy_keys_was_updated =
        !matches!(requested_group_bound_proxy_keys, OptionalField::Missing);
    let requested_group_node_shunt_enabled_was_updated =
        !matches!(requested_group_node_shunt_enabled, OptionalField::Missing);
    let requested_group_note_was_updated = !matches!(requested_group_note, OptionalField::Missing);
    let requested_group_concurrency_limit_was_updated =
        !matches!(requested_concurrency_limit, OptionalField::Missing);

    let display_name = match requested_display_name {
        OptionalField::Missing => session.display_name.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let group_name = match requested_group_name {
        OptionalField::Missing => session.group_name.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let note = match requested_note {
        OptionalField::Missing => session.note.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let session_group_bound_proxy_keys =
        decode_group_bound_proxy_keys_json(session.group_bound_proxy_keys_json.as_deref());
    let session_group_node_shunt_enabled =
        decode_group_node_shunt_enabled(session.group_node_shunt_enabled);
    let session_group_node_shunt_enabled_requested =
        decode_group_requested_flag(session.group_node_shunt_enabled_requested);
    let requested_group_note_missing = matches!(requested_group_note, OptionalField::Missing);
    let mut normalized_group_note = match requested_group_note {
        OptionalField::Missing => session.group_note.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let requested_group_concurrency_limit_missing =
        matches!(requested_concurrency_limit, OptionalField::Missing);
    let mut normalized_group_concurrency_limit = match requested_concurrency_limit {
        OptionalField::Missing => session.group_concurrency_limit,
        OptionalField::Null => 0,
        OptionalField::Value(value) => {
            normalize_concurrency_limit(Some(value), "concurrencyLimit")?
        }
    };
    let group_name_changed = group_name.as_deref() != session.group_name.as_deref();
    let requested_group_bound_proxy_keys = match requested_group_bound_proxy_keys {
        OptionalField::Missing if group_name_changed => None,
        OptionalField::Missing => Some(session_group_bound_proxy_keys.clone()),
        OptionalField::Null => Some(Vec::new()),
        OptionalField::Value(value) => Some(normalize_bound_proxy_keys(value)),
    };
    let requested_group_node_shunt_enabled = match requested_group_node_shunt_enabled {
        OptionalField::Missing if group_name_changed => None,
        OptionalField::Missing => Some(session_group_node_shunt_enabled),
        OptionalField::Null => Some(false),
        OptionalField::Value(value) => Some(value),
    };
    let stored_group_node_shunt_enabled_requested =
        if requested_group_node_shunt_enabled_was_updated {
            true
        } else if group_name_changed {
            false
        } else {
            session_group_node_shunt_enabled_requested
        };
    if requested_group_name_was_updated
        && (group_name.is_none() || (requested_group_note_missing && group_name_changed))
    {
        normalized_group_note = None;
    }
    if requested_group_name_was_updated
        && (group_name.is_none()
            || (requested_group_concurrency_limit_missing && group_name_changed))
    {
        normalized_group_concurrency_limit = 0;
    }
    let mailbox_session_id = match requested_mailbox_session_id {
        OptionalField::Missing => session.mailbox_session_id.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let mailbox_address = match requested_mailbox_address {
        OptionalField::Missing => session.mailbox_address.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let requested_tag_ids = match requested_tag_ids {
        OptionalField::Missing => parse_tag_ids_json(session.tag_ids_json.as_deref()),
        OptionalField::Null => Vec::new(),
        OptionalField::Value(value) => value,
    };
    let tag_ids = validate_tag_ids(&state.pool, &requested_tag_ids).await?;
    let is_mother = match requested_is_mother {
        OptionalField::Missing => session.is_mother != 0,
        OptionalField::Null => false,
        OptionalField::Value(value) => value,
    };
    validate_mailbox_binding(
        &state.pool,
        mailbox_session_id.as_deref(),
        mailbox_address.as_deref(),
    )
    .await?;
    validate_group_note_target(group_name.as_deref(), normalized_group_note.is_some())?;
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        requested_group_bound_proxy_keys,
        requested_group_node_shunt_enabled,
    )
    .await?;
    let tag_ids_json = encode_tag_ids_json(&tag_ids).map_err(internal_error_tuple)?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        normalized_group_note.clone(),
        requested_group_note_was_updated,
        Some(resolved_group_binding.bound_proxy_keys.clone()),
        requested_group_bound_proxy_keys_was_updated,
        normalized_group_concurrency_limit,
        requested_group_concurrency_limit_was_updated,
        Some(resolved_group_binding.node_shunt_enabled),
        requested_group_node_shunt_enabled_was_updated,
    );

    if display_name.as_deref() != session.display_name.as_deref() {
        if let Some(display_name) = display_name.as_deref() {
            ensure_display_name_available(&mut *tx, display_name, session.account_id).await?;
        }
    }

    let stored_group_note = if let Some(group_name) = group_name.as_deref() {
        if normalized_group_note.is_some()
            && group_has_accounts_conn(tx.as_mut(), group_name)
                .await
                .map_err(internal_error_tuple)?
        {
            None
        } else {
            normalized_group_note.clone()
        }
    } else {
        None
    };
    if allows_completed_race_repair {
        let account_id = session.account_id.ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "completed session account is missing".to_string(),
            )
        })?;
        apply_oauth_login_session_metadata_to_account_with_executor(
            &mut tx,
            account_id,
            display_name.clone(),
            Some(resolved_group_binding.group_name.clone()),
            note.clone(),
            &requested_group_metadata_changes,
            is_mother,
            &tag_ids,
        )
        .await?;
        let completed_group_metadata_snapshot = load_group_metadata_snapshot_conn_with_limit(
            tx.as_mut(),
            group_name.as_deref(),
            normalized_group_note.as_deref(),
            normalized_group_concurrency_limit,
        )
        .await
        .map_err(internal_error_tuple)?;
        let now_iso = next_login_session_updated_at(Some(&session.updated_at));
        sqlx::query(
            r#"
            UPDATE pool_oauth_login_sessions
            SET display_name = ?2,
                group_name = ?3,
                group_bound_proxy_keys_json = ?4,
                group_node_shunt_enabled = ?5,
                group_node_shunt_enabled_requested = ?6,
                is_mother = ?7,
                note = ?8,
                tag_ids_json = ?9,
                group_note = ?10,
                group_concurrency_limit = ?11,
                mailbox_session_id = ?12,
                generated_mailbox_address = ?13,
                updated_at = ?14
            WHERE login_id = ?1
            "#,
        )
        .bind(&login_id)
        .bind(display_name)
        .bind(Some(resolved_group_binding.group_name.clone()))
        .bind(
            encode_group_bound_proxy_keys_json(&resolved_group_binding.bound_proxy_keys)
                .map_err(internal_error_tuple)?,
        )
        .bind(if resolved_group_binding.node_shunt_enabled {
            1_i64
        } else {
            0_i64
        })
        .bind(if stored_group_node_shunt_enabled_requested {
            1_i64
        } else {
            0_i64
        })
        .bind(if is_mother { 1 } else { 0 })
        .bind(note)
        .bind(&tag_ids_json)
        .bind(completed_group_metadata_snapshot.note)
        .bind(completed_group_metadata_snapshot.concurrency_limit)
        .bind(mailbox_session_id)
        .bind(mailbox_address)
        .bind(&now_iso)
        .execute(&mut *tx)
        .await
        .map_err(internal_error_tuple)?;
        let updated = load_login_session_by_login_id_with_executor(&mut *tx, &login_id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
        tx.commit().await.map_err(internal_error_tuple)?;
        return Ok(Json(login_session_to_response_with_sync_applied(
            &updated, true,
        )));
    }
    let now_iso = next_login_session_updated_at(Some(&session.updated_at));
    let result = sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET display_name = ?2,
            group_name = ?3,
            group_bound_proxy_keys_json = ?4,
            group_node_shunt_enabled = ?5,
            group_node_shunt_enabled_requested = ?6,
            is_mother = ?7,
            note = ?8,
            tag_ids_json = ?9,
            group_note = ?10,
            group_concurrency_limit = ?11,
            mailbox_session_id = ?12,
            generated_mailbox_address = ?13,
            updated_at = ?14
        WHERE login_id = ?1
          AND (?15 IS NULL OR updated_at = ?15)
        "#,
    )
    .bind(&login_id)
    .bind(display_name)
    .bind(Some(resolved_group_binding.group_name.clone()))
    .bind(
        encode_group_bound_proxy_keys_json(&resolved_group_binding.bound_proxy_keys)
            .map_err(internal_error_tuple)?,
    )
    .bind(if resolved_group_binding.node_shunt_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(if stored_group_node_shunt_enabled_requested {
        1_i64
    } else {
        0_i64
    })
    .bind(if is_mother { 1 } else { 0 })
    .bind(note)
    .bind(tag_ids_json)
    .bind(stored_group_note)
    .bind(normalized_group_concurrency_limit)
    .bind(mailbox_session_id)
    .bind(mailbox_address)
    .bind(&now_iso)
    .bind(requested_base_updated_at.as_deref())
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    let updated = load_login_session_by_login_id_with_executor(&mut *tx, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    if result.rows_affected() == 0 {
        tx.commit().await.map_err(internal_error_tuple)?;
        return Ok(Json(login_session_to_response_with_sync_applied(
            &updated, false,
        )));
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(Json(login_session_to_response_with_sync_applied(
        &updated, true,
    )))
}

pub(crate) async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OauthCallbackQuery>,
) -> Response {
    match handle_oauth_callback(state, query).await {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err((status, html)) => (status, Html(html)).into_response(),
    }
}

pub(crate) async fn complete_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(login_id): AxumPath<String>,
    Json(payload): Json<CompleteOauthLoginSessionRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id(&state.pool, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    validate_mailbox_binding_fields(
        payload.mailbox_session_id.as_deref(),
        payload.mailbox_address.as_deref(),
    )?;
    if session.mailbox_session_id.as_deref() != payload.mailbox_session_id.as_deref()
        || !mailbox_addresses_match(
            session.mailbox_address.as_deref(),
            payload.mailbox_address.as_deref(),
        )
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "mailbox binding no longer matches this OAuth login session".to_string(),
        ));
    }
    validate_mailbox_binding(
        &state.pool,
        session.mailbox_session_id.as_deref(),
        session.mailbox_address.as_deref(),
    )
    .await?;
    let query = parse_manual_oauth_callback(&payload.callback_url, &session.redirect_uri)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    let account_id = complete_oauth_login_session_with_query(state.clone(), session, query).await?;
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "account not found after oauth completion".to_string(),
            )
        })?;
    Ok(Json(detail))
}

pub(crate) async fn relogin_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    let tag_ids = load_account_tag_map(&state.pool, &[id])
        .await
        .map_err(internal_error_tuple)?
        .remove(&id)
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.id)
        .collect();
    let payload = CreateOauthLoginSessionRequest {
        display_name: None,
        group_name: None,
        group_bound_proxy_keys: None,
        group_node_shunt_enabled: None,
        note: None,
        group_note: None,
        concurrency_limit: None,
        account_id: Some(id),
        tag_ids,
        is_mother: None,
        mailbox_session_id: None,
        mailbox_address: None,
    };
    create_oauth_login_session(State(state), headers, Json(payload)).await
}

pub(crate) async fn apply_mother_assignment(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    group_name: Option<&str>,
    is_mother: bool,
) -> Result<()> {
    if is_mother {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET is_mother = 0
            WHERE id != ?1
              AND COALESCE(group_name, '') = COALESCE(?2, '')
              AND is_mother != 0
            "#,
        )
        .bind(account_id)
        .bind(group_name)
        .execute(&mut **tx)
        .await?;
    }

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET is_mother = ?2
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(if is_mother { 1 } else { 0 })
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub(crate) async fn create_api_key_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateApiKeyAccountRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let detail = create_api_key_account_inner(state, payload).await?;
    Ok(Json(detail))
}

pub(crate) async fn create_api_key_account_inner(
    state: Arc<AppState>,
    payload: CreateApiKeyAccountRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let display_name = normalize_required_display_name(&payload.display_name)?;
    validate_local_limits(payload.local_primary_limit, payload.local_secondary_limit)?;
    let api_key = normalize_required_secret(&payload.api_key, "apiKey")?;
    let tag_ids = validate_tag_ids(&state.pool, &payload.tag_ids).await?;
    let group_name = normalize_optional_text(payload.group_name);
    let note = normalize_optional_text(payload.note);
    let has_group_note = payload.group_note.is_some();
    let group_note = normalize_optional_text(payload.group_note);
    let group_concurrency_limit =
        normalize_concurrency_limit(payload.concurrency_limit, "concurrencyLimit")?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        group_note.clone(),
        has_group_note,
        payload.group_bound_proxy_keys.clone(),
        payload.group_bound_proxy_keys.is_some(),
        group_concurrency_limit,
        payload.concurrency_limit.is_some(),
        payload.group_node_shunt_enabled,
        payload.group_node_shunt_enabled.is_some(),
    );
    validate_group_note_target(group_name.as_deref(), has_group_note)?;
    let resolved_group_binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        group_name.clone(),
        payload.group_bound_proxy_keys,
        payload.group_node_shunt_enabled,
    )
    .await?;
    let target_group_name = Some(resolved_group_binding.group_name.clone());
    let is_mother = payload.is_mother.unwrap_or(false);
    let limit_unit = normalize_limit_unit(payload.local_limit_unit);
    let upstream_base_url = normalize_optional_upstream_base_url(payload.upstream_base_url)?;
    let masked_api_key = mask_api_key(&api_key);
    let now_iso = format_utc_iso(Utc::now());
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::ApiKey(StoredApiKeyCredentials { api_key }),
    )
    .map_err(internal_error_tuple)?;
    let inserted_id = {
        let mut tx = state
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(internal_error_tuple)?;
        ensure_display_name_available(&mut *tx, &display_name, None).await?;
        let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_upstream_accounts (
            kind, provider, display_name, group_name, is_mother, note, status, enabled, email, chatgpt_account_id,
            chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key, encrypted_credentials, token_expires_at,
            last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, NULL, NULL,
            NULL, NULL, NULL, ?8, ?9, NULL,
            NULL, NULL, NULL, NULL, NULL,
            ?10, ?11, ?12, ?13, ?14, ?14
        ) RETURNING id
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
    .bind(display_name)
    .bind(&target_group_name)
    .bind(if is_mother { 1 } else { 0 })
    .bind(note)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(masked_api_key)
    .bind(encrypted_credentials)
    .bind(payload.local_primary_limit)
    .bind(payload.local_secondary_limit)
    .bind(limit_unit)
    .bind(upstream_base_url)
    .bind(&now_iso)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
        apply_mother_assignment(&mut tx, inserted_id, group_name.as_deref(), is_mother)
            .await
            .map_err(internal_error_tuple)?;

        save_group_metadata_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            &requested_group_metadata_changes,
            false,
        )
        .await
        .map_err(internal_error_tuple)?;
        tx.commit().await.map_err(internal_error_tuple)?;
        inserted_id
    };

    sync_account_tag_links(&state.pool, inserted_id, &tag_ids)
        .await
        .map_err(internal_error_tuple)?;
    let detail = state
        .upstream_accounts
        .account_ops
        .run_post_create_sync(state.clone(), inserted_id)
        .await
        .map_err(request_runtime_error_tuple)?;
    Ok(detail)
}

pub(crate) async fn update_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateUpstreamAccountRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(state.clone(), id, payload)
        .await?;
    Ok(Json(detail))
}

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
    let clear_hard_failure_after_update = row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
        && account_update_requests_manual_recovery(&payload)
        && route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        );
    let tag_ids = match payload.tag_ids.as_ref() {
        Some(values) => Some(validate_tag_ids(&state.pool, values).await?),
        None => None,
    };
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
    );

    if let Some(display_name) = payload.display_name {
        row.display_name = normalize_required_display_name(&display_name)?;
    }
    if let Some(group_name) = payload.group_name.clone() {
        row.group_name = normalize_optional_text(Some(group_name));
    }
    if let Some(note) = payload.note {
        row.note = normalize_optional_text(Some(note));
    }
    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
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

    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
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
    validate_group_note_target(row.group_name.as_deref(), requested_group_note.is_some())?;
    let resolved_group_binding = if payload.group_name.is_some()
        || payload.group_bound_proxy_keys.is_some()
        || payload.group_node_shunt_enabled.is_some()
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
    let now_iso = format_utc_iso(Utc::now());
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    ensure_display_name_available(&mut *tx, &row.display_name, Some(id)).await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            group_name = ?3,
            is_mother = ?4,
            note = ?5,
            enabled = ?6,
            masked_api_key = ?7,
            encrypted_credentials = ?8,
            local_primary_limit = ?9,
            local_secondary_limit = ?10,
            local_limit_unit = ?11,
            upstream_base_url = ?12,
            updated_at = ?13
        WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(&row.display_name)
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
    .bind(&now_iso)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;
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
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Some(tag_ids) = tag_ids {
        sync_account_tag_links(&state.pool, id, &tag_ids)
            .await
            .map_err(internal_error_tuple)?;
    }
    if clear_hard_failure_after_update {
        set_account_status(&state.pool, id, UPSTREAM_ACCOUNT_STATUS_ACTIVE, None)
            .await
            .map_err(internal_error_tuple)?;
    }
    record_account_update_action(&state.pool, id, "account settings were updated")
        .await
        .map_err(internal_error_tuple)?;

    let detail = load_upstream_account_detail_with_actual_usage(state, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(detail)
}

pub(crate) async fn apply_oauth_login_session_metadata_to_account_with_executor(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    display_name: Option<String>,
    group_name: Option<String>,
    note: Option<String>,
    requested_group_metadata_changes: &RequestedGroupMetadataChanges,
    is_mother: bool,
    tag_ids: &[i64],
) -> Result<(), (StatusCode, String)> {
    let row = load_upstream_account_row_conn(tx.as_mut(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let previous_group_name = row.group_name.clone();
    let next_display_name = display_name.unwrap_or(row.display_name);
    ensure_display_name_available(tx.as_mut(), &next_display_name, Some(account_id)).await?;

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            group_name = ?3,
            is_mother = ?4,
            note = ?5,
            updated_at = ?6
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(&next_display_name)
    .bind(&group_name)
    .bind(if is_mother { 1 } else { 0 })
    .bind(&note)
    .bind(&now_iso)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;

    save_group_metadata_after_account_write(
        tx.as_mut(),
        group_name.as_deref(),
        requested_group_metadata_changes,
        previous_group_name == group_name,
    )
    .await
    .map_err(internal_error_tuple)?;
    if previous_group_name != group_name {
        cleanup_orphaned_group_metadata(tx.as_mut(), previous_group_name.as_deref())
            .await
            .map_err(internal_error_tuple)?;
    }
    apply_mother_assignment(tx, account_id, group_name.as_deref(), is_mother)
        .await
        .map_err(internal_error_tuple)?;
    sync_account_tag_links_with_executor(tx.as_mut(), account_id, tag_ids)
        .await
        .map_err(internal_error_tuple)?;
    Ok(())
}

pub(crate) async fn delete_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let status = state
        .upstream_accounts
        .account_ops
        .run_delete_account(state.clone(), id)
        .await?;
    Ok(status)
}

pub(crate) async fn delete_upstream_account_inner(
    state: &AppState,
    id: i64,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let group_name = load_upstream_account_row_conn(tx.as_mut(), id)
        .await
        .map_err(internal_error_tuple)?
        .map(|row| row.group_name);
    sqlx::query("DELETE FROM pool_upstream_account_limit_samples WHERE account_id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    sqlx::query("DELETE FROM pool_upstream_account_tags WHERE account_id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    sqlx::query("DELETE FROM pool_oauth_login_sessions WHERE account_id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    let affected = sqlx::query("DELETE FROM pool_upstream_accounts WHERE id = ?1")
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?
        .rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    cleanup_orphaned_group_metadata(
        tx.as_mut(),
        group_name.as_ref().and_then(|value| value.as_deref()),
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn sync_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let detail = state
        .upstream_accounts
        .account_ops
        .run_manual_sync(state.clone(), id)
        .await
        .map_err(request_runtime_error_tuple)?;
    Ok(Json(detail))
}

pub(crate) async fn handle_oauth_callback(
    state: Arc<AppState>,
    query: OauthCallbackQuery,
) -> Result<String, (StatusCode, String)> {
    let Some(state_value) = normalize_optional_text(query.state.clone()) else {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(false, "OAuth callback rejected", "Missing state parameter."),
        ));
    };

    let Some(session) = load_login_session_by_state(&state.pool, &state_value)
        .await
        .map_err(internal_error_html)?
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(
                false,
                "OAuth callback rejected",
                "Login session was not found.",
            ),
        ));
    };

    complete_oauth_login_session_with_query(state, session, query)
        .await
        .map_err(|(status, message)| {
            let title = match status {
                StatusCode::BAD_GATEWAY => "OAuth token exchange failed",
                StatusCode::SERVICE_UNAVAILABLE => "Credential storage disabled",
                _ if message.contains("expired") => "OAuth callback expired",
                _ if message.contains("authorization failed") => "OAuth authorization failed",
                _ => "OAuth callback rejected",
            };
            (status, render_callback_page(false, title, &message))
        })?;

    Ok(render_callback_page(
        true,
        "OAuth login complete",
        "The upstream account is ready. You can close this window.",
    ))
}

pub(crate) async fn complete_oauth_login_session_with_query(
    state: Arc<AppState>,
    session: OauthLoginSessionRow,
    query: OauthCallbackQuery,
) -> Result<i64, (StatusCode, String)> {
    let now = Utc::now();
    let Some(expires_at) = parse_rfc3339_utc(&session.expires_at) else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Stored session expiry is invalid.".to_string(),
        ));
    };
    if session.status != LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session has already been consumed.".to_string(),
        ));
    }
    if now > expires_at {
        mark_login_session_expired(&state.pool, &session.login_id)
            .await
            .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_REQUEST,
            "The login session has expired. Please create a new authorization link.".to_string(),
        ));
    }

    let callback_state = normalize_optional_text(query.state.clone()).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing state parameter.".to_string(),
        )
    })?;
    if callback_state != session.state {
        return Err((
            StatusCode::BAD_REQUEST,
            "The callback URL does not belong to this login session.".to_string(),
        ));
    }

    if let Some(error) = normalize_optional_text(query.error) {
        let detail = normalize_optional_text(query.error_description)
            .unwrap_or_else(|| "Authorization was cancelled or rejected.".to_string());
        fail_login_session(
            &state.pool,
            &session.login_id,
            &format!("{error}: {detail}"),
        )
        .await
        .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_REQUEST,
            format!("OAuth authorization failed: {detail}"),
        ));
    }

    let code = normalize_optional_text(query.code).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing authorization code.".to_string(),
        )
    })?;

    let session_scope = login_session_required_forward_proxy_scope(&session)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    let token_response = exchange_authorization_code_for_required_scope(
        state.as_ref(),
        &session_scope,
        &code,
        &session.pkce_verifier,
        &session.redirect_uri,
    )
    .await
    .map_err(|err| (StatusCode::BAD_GATEWAY, err.to_string()))?;

    let Some(id_token) = token_response.id_token.clone() else {
        fail_login_session(
            &state.pool,
            &session.login_id,
            "id_token missing in token exchange response",
        )
        .await
        .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            "The token response did not include an id_token.".to_string(),
        ));
    };
    let Some(refresh_token) = token_response.refresh_token.clone() else {
        fail_login_session(
            &state.pool,
            &session.login_id,
            "refresh_token missing in token exchange response",
        )
        .await
        .map_err(internal_error_tuple)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            "The token response did not include a refresh token.".to_string(),
        ));
    };

    let claims = parse_chatgpt_jwt_claims(&id_token)
        .map_err(|err| (StatusCode::BAD_GATEWAY, err.to_string()))?;
    let crypto_key = state.upstream_accounts.crypto_key.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "{} is required to persist OAuth credentials.",
                ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
            ),
        )
    })?;

    let token_expires_at =
        format_utc_iso(Utc::now() + ChronoDuration::seconds(token_response.expires_in.max(0)));
    let credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: token_response.access_token.clone(),
            refresh_token,
            id_token,
            token_type: token_response.token_type.clone(),
        }),
    )
    .map_err(internal_error_tuple)?;

    let default_display_name = claims
        .email
        .clone()
        .or_else(|| session.display_name.clone())
        .unwrap_or_else(|| "Codex OAuth".to_string());
    let display_name = session
        .display_name
        .clone()
        .and_then(|value| normalize_optional_text(Some(value)))
        .unwrap_or(default_display_name);
    let input = PersistOauthCallbackInput {
        session,
        display_name,
        claims,
        encrypted_credentials: credentials,
        token_expires_at,
    };
    let account_id = if let Some(existing_account_id) = input.session.account_id {
        state
            .upstream_accounts
            .account_ops
            .run_persist_oauth_callback(state.clone(), existing_account_id, input)
            .await?
    } else {
        let account_id = persist_new_oauth_callback_inner(state.as_ref(), input).await?;
        if let Err(err) = state
            .upstream_accounts
            .account_ops
            .run_post_create_sync(state.clone(), account_id)
            .await
        {
            warn!(account_id, error = %err, "OAuth callback created account but initial sync failed");
        }
        account_id
    };

    Ok(account_id)
}

pub(crate) async fn persist_existing_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    let account_id = persist_oauth_callback_inner(state, input).await?;
    if let Err(err) = sync_upstream_account_by_id(state, account_id, SyncCause::PostCreate).await {
        warn!(account_id, error = %err, "OAuth callback updated account but initial sync failed");
    }
    Ok(account_id)
}

pub(crate) async fn persist_new_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    persist_oauth_callback_inner(state, input).await
}

pub(crate) async fn persist_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id_with_executor(&mut *tx, &input.session.login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    if session.status != LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session has already been consumed.".to_string(),
        ));
    }
    if let Err((status, message)) =
        ensure_display_name_available(&mut *tx, &input.display_name, session.account_id).await
    {
        if status == StatusCode::CONFLICT {
            fail_login_session_with_executor(&mut *tx, &session.login_id, &message)
                .await
                .map_err(internal_error_tuple)?;
            tx.commit().await.map_err(internal_error_tuple)?;
        }
        return Err((status, message));
    }
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: session.account_id,
            display_name: &input.display_name,
            group_name: session.group_name.clone(),
            is_mother: session.is_mother != 0,
            note: session.note.clone(),
            tag_ids: parse_tag_ids_json(session.tag_ids_json.as_deref()),
            requested_group_metadata_changes: build_requested_group_metadata_changes(
                session.group_note.clone(),
                true,
                Some(decode_group_bound_proxy_keys_json(
                    session.group_bound_proxy_keys_json.as_deref(),
                )),
                true,
                session.group_concurrency_limit,
                true,
                Some(decode_group_node_shunt_enabled(
                    session.group_node_shunt_enabled,
                )),
                decode_group_requested_flag(session.group_node_shunt_enabled_requested),
            ),
            claims: &input.claims,
            encrypted_credentials: input.encrypted_credentials,
            token_expires_at: &input.token_expires_at,
            external_identity: None,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    let completed_group_metadata_snapshot = load_group_metadata_snapshot_conn_with_limit(
        tx.as_mut(),
        session.group_name.as_deref(),
        session.group_note.as_deref(),
        session.group_concurrency_limit,
    )
    .await
    .map_err(internal_error_tuple)?;
    complete_login_session_with_executor(
        &mut *tx,
        &session.login_id,
        account_id,
        completed_group_metadata_snapshot.note,
        completed_group_metadata_snapshot.concurrency_limit,
        &session.updated_at,
        session.account_id.is_none(),
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(account_id)
}

pub(crate) fn parse_manual_oauth_callback(
    callback_url: &str,
    expected_redirect_uri: &str,
) -> Result<OauthCallbackQuery> {
    let trimmed = callback_url.trim();
    if trimmed.is_empty() {
        bail!("Callback URL is required.");
    }

    let expected =
        Url::parse(expected_redirect_uri).context("failed to parse stored redirect URI")?;
    let parsed = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Url::parse(trimmed).context("callback URL must be a valid absolute URL")?
    } else if trimmed.starts_with('?') || trimmed.contains("code=") || trimmed.contains("state=") {
        let mut url = expected.clone();
        let query = trimmed.strip_prefix('?').unwrap_or(trimmed);
        url.set_query(Some(query));
        url
    } else {
        bail!("Callback URL must be a full URL or query string.");
    };

    if parsed.scheme() != expected.scheme()
        || parsed.host_str() != expected.host_str()
        || parsed.port_or_known_default() != expected.port_or_known_default()
        || parsed.path() != expected.path()
    {
        bail!("Callback URL does not match the generated localhost redirect address.");
    }

    let mut query = OauthCallbackQuery {
        code: None,
        state: None,
        error: None,
        error_description: None,
    };
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "code" if query.code.is_none() => query.code = Some(value.into_owned()),
            "state" if query.state.is_none() => query.state = Some(value.into_owned()),
            "error" if query.error.is_none() => query.error = Some(value.into_owned()),
            "error_description" if query.error_description.is_none() => {
                query.error_description = Some(value.into_owned())
            }
            _ => {}
        }
    }
    Ok(query)
}
