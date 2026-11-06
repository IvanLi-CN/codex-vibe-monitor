use super::*;

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
        let kaisoumail_meta = kaisoumail_get_meta(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?;
        let supported_domains = kaisoumail_supported_domains(&kaisoumail_meta);
        let email_domain = manual_email_address
            .split('@')
            .nth(1)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !kaisoumail_domain_is_supported(&email_domain, &supported_domains) {
            return Ok(Json(oauth_mailbox_session_unsupported_response(
                manual_email_address,
                "unsupported_domain",
            )));
        }
        let existing_remote_mailbox = kaisoumail_list_mailboxes(&state.http_clients.shared, config)
            .await
            .map_err(internal_error_tuple)?
            .into_iter()
            .find(|item| {
                normalize_mailbox_address(&item.address) == Some(manual_email_address.clone())
            });
        let Some(remote_mailbox) = existing_remote_mailbox else {
            let generated = kaisoumail_ensure_mailbox_for_address(
                &state.http_clients.shared,
                config,
                &manual_email_address,
            )
            .await
            .map_err(internal_error_tuple)?;
            let email_address = generated.address.trim().to_string();
            let email_domain = email_address
                .split('@')
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let session_id = random_hex(16)?;
            let now = Utc::now();
            let expires_at = now
                + ChronoDuration::seconds(
                    DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64,
                );
            let now_iso = format_utc_iso(now);
            let session_expires_at =
                normalize_mailbox_session_expires_at(generated.expires_at.as_deref(), expires_at);
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
            .bind(&session_expires_at)
            .execute(&state.pool)
            .await
            .map_err(internal_error_tuple)?;

            return Ok(Json(oauth_mailbox_session_supported_response(
                session_id,
                email_address,
                session_expires_at,
                OAUTH_MAILBOX_SOURCE_GENERATED,
            )));
        };
        let session_id = random_hex(16)?;
        let now = Utc::now();
        let mut remote_mailbox_id = remote_mailbox.id;
        let mut remote_mailbox_expires_at = remote_mailbox.expires_at;
        if mailbox_expires_at_is_expired(remote_mailbox_expires_at.as_deref(), now) {
            let restored = kaisoumail_ensure_mailbox_for_address(
                &state.http_clients.shared,
                config,
                &manual_email_address,
            )
            .await
            .map_err(internal_error_tuple)?;
            remote_mailbox_id = restored.id;
            remote_mailbox_expires_at = restored.expires_at;
        }
        let mut remote_messages = match kaisoumail_list_messages_for_attach(
            &state.http_clients.shared,
            config,
            &manual_email_address,
        )
        .await
        .map_err(internal_error_tuple)?
        {
            KaisouMailAttachReadState::Readable(messages) => messages,
            KaisouMailAttachReadState::NotReadable => {
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
            &remote_messages,
        )
        .await
        .map_err(internal_error_tuple)?
        {
            KaisouMailAttachReadState::Readable(state) => state,
            KaisouMailAttachReadState::NotReadable => {
                return Ok(Json(oauth_mailbox_session_unsupported_response(
                    manual_email_address,
                    "not_readable",
                )));
            }
        };
        let expires_at = normalize_mailbox_session_expires_at(
            remote_mailbox_expires_at.as_deref(),
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
        .bind(&remote_mailbox_id)
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
    let generated = kaisoumail_create_mailbox(&state.http_clients.shared, config)
        .await
        .map_err(internal_error_tuple)?;
    let email_address = generated.address.trim().to_string();
    let email_domain = email_address
        .split('@')
        .nth(1)
        .unwrap_or_default()
        .to_string();
    let session_id = random_hex(16)?;
    let now = Utc::now();
    let expires_at =
        now + ChronoDuration::seconds(DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS as i64);
    let now_iso = format_utc_iso(now);
    let session_expires_at =
        normalize_mailbox_session_expires_at(generated.expires_at.as_deref(), expires_at);
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
    .bind(&session_expires_at)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple)?;

    Ok(Json(oauth_mailbox_session_supported_response(
        session_id,
        email_address,
        session_expires_at,
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
        && let Some(config) = state.config.upstream_accounts_kaisoumail.as_ref()
        && let Err(err) =
            kaisoumail_delete_mailbox(&state.http_clients.shared, config, &row.remote_email_id)
                .await
    {
        debug!(
            mailbox_session_id = %row.session_id,
            remote_email_id = %row.remote_email_id,
            error = %err,
            "failed to delete kaisoumail mailbox during explicit cleanup"
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
    reject_manual_tag_ids(&payload.tag_ids)?;
    let tag_ids = Vec::new();
    let tag_ids_json = encode_tag_ids_json(&tag_ids).map_err(internal_error_tuple)?;

    let mut preserved_mother_flag = false;
    let mut preserved_account_updated_at = None;
    let mut preserved_display_name = None;
    let mut preserved_email = None;
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
        preserved_account_updated_at = Some(existing.updated_at);
        preserved_display_name = Some(existing.display_name);
        preserved_email = existing.email;
        preserved_group_name = existing.group_name;
        preserved_note = existing.note;
        preserved_group_concurrency_limit =
            load_group_metadata(&state.pool, preserved_group_name.as_deref())
                .await
                .map_err(internal_error_tuple)?
                .concurrency_limit;
    }

    let is_mother = payload.is_mother.unwrap_or(preserved_mother_flag);
    let requested_email = normalize_optional_email(payload.email, "email")?;
    let email = requested_email.clone().or_else(|| preserved_email.clone());
    let requested_display_name = normalize_optional_text(payload.display_name);
    let display_name = resolve_display_name_after_email_change(
        preserved_display_name.as_deref(),
        requested_display_name.as_deref(),
        preserved_email.as_deref(),
        email.as_deref(),
    );
    let group_name = Some(normalize_upstream_account_group_name(
        normalize_optional_text(payload.group_name).or(preserved_group_name),
    ));
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

    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, account_updated_at, display_name, email, group_name, group_bound_proxy_keys_json, group_node_shunt_enabled,
            group_node_shunt_enabled_requested, group_single_account_rotation_enabled,
            group_single_account_rotation_enabled_requested, is_mother, note, tag_ids_json, group_note, group_concurrency_limit,
            mailbox_session_id, generated_mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, expires_at, consumed_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, NULL, ?24, NULL, ?25, ?25)
        "#,
    )
    .bind(&login_id)
    .bind(payload.account_id)
    .bind(preserved_account_updated_at)
    .bind(display_name)
    .bind(email.clone())
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
    .bind(if payload
        .group_single_account_rotation_enabled
        .unwrap_or(false)
    {
        1_i64
    } else {
        0_i64
    })
    .bind(if payload.group_single_account_rotation_enabled.is_some() {
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
        email,
        error: None,
        sync_applied: None,
        identity_confirmation: None,
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
        let session_group_node_shunt_enabled_requested =
            decode_group_requested_flag(session.group_node_shunt_enabled_requested);
        session.display_name.as_deref() == Some(account.display_name.as_str())
            && session.email == account.email
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
    if session.status == LOGIN_SESSION_STATUS_PENDING
        && let Some(requested_base_updated_at) = requested_base_updated_at.as_deref()
        && requested_base_updated_at != session.updated_at
    {
        tx.commit().await.map_err(internal_error_tuple)?;
        return Ok(Json(login_session_to_response_with_sync_applied(
            &session, false,
        )));
    }

    let UpdateOauthLoginSessionRequest {
        display_name: requested_display_name,
        email: requested_email,
        group_name: requested_group_name,
        group_bound_proxy_keys: requested_group_bound_proxy_keys,
        group_node_shunt_enabled: requested_group_node_shunt_enabled,
        group_single_account_rotation_enabled: requested_group_single_account_rotation_enabled,
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
    let requested_group_single_account_rotation_enabled_was_updated = !matches!(
        requested_group_single_account_rotation_enabled,
        OptionalField::Missing
    );
    let requested_group_note_was_updated = !matches!(requested_group_note, OptionalField::Missing);
    let requested_group_concurrency_limit_was_updated =
        !matches!(requested_concurrency_limit, OptionalField::Missing);

    let display_name = match requested_display_name {
        OptionalField::Missing => session.display_name.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_text(Some(value)),
    };
    let email = match requested_email {
        OptionalField::Missing => session.email.clone(),
        OptionalField::Null => None,
        OptionalField::Value(value) => normalize_optional_email(Some(value), "email")?,
    };
    let group_name = match requested_group_name {
        OptionalField::Missing => session.group_name.clone(),
        OptionalField::Null => Some(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME.to_string()),
        OptionalField::Value(value) => Some(normalize_upstream_account_group_name(Some(value))),
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
    let session_group_single_account_rotation_enabled =
        decode_group_single_account_rotation_enabled(session.group_single_account_rotation_enabled);
    let session_group_single_account_rotation_enabled_requested =
        decode_group_requested_flag(session.group_single_account_rotation_enabled_requested);
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
    let requested_group_single_account_rotation_enabled =
        match requested_group_single_account_rotation_enabled {
            OptionalField::Missing if group_name_changed => None,
            OptionalField::Missing => Some(session_group_single_account_rotation_enabled),
            OptionalField::Null => Some(false),
            OptionalField::Value(value) => Some(value),
        };
    let stored_group_single_account_rotation_enabled_requested =
        if requested_group_single_account_rotation_enabled_was_updated {
            true
        } else if group_name_changed {
            false
        } else {
            session_group_single_account_rotation_enabled_requested
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
        OptionalField::Missing => Vec::new(),
        OptionalField::Null => Vec::new(),
        OptionalField::Value(value) => value,
    };
    reject_manual_tag_ids(&requested_tag_ids)?;
    let tag_ids = Vec::new();
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
        requested_group_single_account_rotation_enabled,
        requested_group_single_account_rotation_enabled_was_updated,
    );

    let next_display_name = resolve_display_name_after_email_change(
        session.display_name.as_deref(),
        display_name.as_deref(),
        session.email.as_deref(),
        email.as_deref(),
    );

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
            next_display_name.clone(),
            email.clone(),
            Some(resolved_group_binding.group_name.clone()),
            note.clone(),
            &requested_group_metadata_changes,
            is_mother,
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
                email = ?3,
                group_name = ?4,
                group_bound_proxy_keys_json = ?5,
                group_node_shunt_enabled = ?6,
                group_node_shunt_enabled_requested = ?7,
                group_single_account_rotation_enabled = ?8,
                group_single_account_rotation_enabled_requested = ?9,
                is_mother = ?10,
                note = ?11,
                tag_ids_json = ?12,
                group_note = ?13,
                group_concurrency_limit = ?14,
                mailbox_session_id = ?15,
                generated_mailbox_address = ?16,
                updated_at = ?17
            WHERE login_id = ?1
            "#,
        )
        .bind(&login_id)
        .bind(next_display_name)
        .bind(email)
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
        .bind(
            if requested_group_single_account_rotation_enabled.unwrap_or(false) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(if stored_group_single_account_rotation_enabled_requested {
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
            email = ?3,
            group_name = ?4,
            group_bound_proxy_keys_json = ?5,
            group_node_shunt_enabled = ?6,
            group_node_shunt_enabled_requested = ?7,
            group_single_account_rotation_enabled = ?8,
            group_single_account_rotation_enabled_requested = ?9,
            is_mother = ?10,
            note = ?11,
            tag_ids_json = ?12,
            group_note = ?13,
            group_concurrency_limit = ?14,
            mailbox_session_id = ?15,
            generated_mailbox_address = ?16,
            updated_at = ?17
        WHERE login_id = ?1
          AND (?18 IS NULL OR updated_at = ?18)
        "#,
    )
    .bind(&login_id)
    .bind(next_display_name)
    .bind(email)
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
    .bind(
        if requested_group_single_account_rotation_enabled.unwrap_or(false) {
            1_i64
        } else {
            0_i64
        },
    )
    .bind(if stored_group_single_account_rotation_enabled_requested {
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
