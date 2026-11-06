use super::*;

pub(crate) async fn confirm_oauth_login_session_identity_overwrite(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(login_id): AxumPath<String>,
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
    let account_id = state
        .upstream_accounts
        .account_ops
        .run_confirm_oauth_identity_overwrite(state.clone(), login_id)
        .await?;
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "account not found after oauth confirmation".to_string(),
            )
        })?;
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
            let safe_message = match status {
                StatusCode::BAD_GATEWAY => {
                    "The upstream OAuth token exchange failed. Check server logs for details."
                }
                StatusCode::SERVICE_UNAVAILABLE => {
                    "Credential storage is unavailable. Check server configuration."
                }
                _ if message.contains("expired") => {
                    "The OAuth callback expired. Please create a new authorization link."
                }
                _ if message.contains("authorization failed") => {
                    "The OAuth authorization was rejected or cancelled."
                }
                _ => "The OAuth callback was rejected. Check server logs for details.",
            };
            (status, render_callback_page(false, title, safe_message))
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
    let claims = parse_chatgpt_jwt_claims(&id_token)
        .map_err(|err| (StatusCode::BAD_GATEWAY, err.to_string()))?;
    let token_lifecycle_metadata = oauth_token_response_lifecycle_metadata(
        &token_response,
        &token_response.access_token,
        Utc::now(),
    );
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
            refresh_token: normalize_oauth_refresh_token(token_response.refresh_token.clone()),
            id_token,
            token_type: token_response.token_type.clone(),
        }),
    )
    .map_err(internal_error_tuple)?;

    let normalized_claim_email = normalize_email_value(claims.email.clone());
    let default_display_name = session
        .display_name
        .clone()
        .and_then(|value| normalize_optional_text(Some(value)))
        .or_else(|| generated_display_name_from_email(session.email.as_deref()))
        .or_else(|| generated_display_name_from_email(normalized_claim_email.as_deref()))
        .unwrap_or_else(|| "Codex OAuth".to_string());
    let display_name = session
        .display_name
        .clone()
        .and_then(|value| normalize_optional_text(Some(value)))
        .unwrap_or(default_display_name);
    let chosen_email = session.email.clone();
    let is_new_account = session.account_id.is_none();
    let input = PersistOauthCallbackInput {
        session,
        display_name,
        chosen_email,
        verified_email: normalized_claim_email,
        claims,
        encrypted_credentials: credentials,
        has_refresh_token: normalize_oauth_refresh_token(token_response.refresh_token).is_some(),
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
        publish_new_account_routing_availability_if_selectable(state.as_ref(), account_id).await;
        account_id
    };

    persist_oauth_token_lifecycle_metadata(&state.pool, account_id, &token_lifecycle_metadata)
        .await
        .map_err(internal_error_tuple)?;

    if is_new_account
        && let Err(err) = publish_account_effective_routing_rules_changed(
            state.as_ref(),
            Some(&[account_id]),
            &[],
        )
        .await
    {
        warn!(
            ?err,
            account_id, "OAuth callback committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }

    Ok(account_id)
}

pub(crate) async fn persist_existing_oauth_callback_inner(
    state: &AppState,
    input: PersistOauthCallbackInput,
) -> Result<i64, (StatusCode, String)> {
    let account_id = if let Some(account_id) = input.session.account_id {
        let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(account_id);
        let _credential_refresh_guard = credential_refresh_lock.lock().await;
        persist_oauth_callback_inner(state, input).await?
    } else {
        persist_oauth_callback_inner(state, input).await?
    };
    if let Err(err) = sync_upstream_account_by_id(state, account_id, SyncCause::PostCreate).await {
        warn!(account_id, error = %err, "OAuth callback updated account but initial sync failed");
    }
    if let Err(err) =
        publish_account_effective_routing_rules_changed(state, Some(&[account_id]), &[]).await
    {
        warn!(
            ?err,
            account_id, "OAuth callback update committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
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
    let credential_refresh_next_attempt_at =
        parse_rfc3339_utc(&input.token_expires_at).map(|expires| {
            format_utc_iso(
                expires
                    - ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    ),
            )
        });
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
    let existing_account = if let Some(account_id) = session.account_id {
        let account = load_upstream_account_row_conn(tx.as_mut(), account_id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
        if oauth_identity_requires_confirmation(&account, &input) {
            mark_login_session_needs_identity_confirmation_with_executor(
                &mut *tx, &session, &input,
            )
            .await
            .map_err(internal_error_tuple)?;
            tx.commit().await.map_err(internal_error_tuple)?;
            return Err((
                StatusCode::CONFLICT,
                "OAuth identity confirmation required".to_string(),
            ));
        }
        if session
            .account_updated_at
            .as_deref()
            .is_some_and(|expected| expected != account.updated_at)
        {
            return Err((
                StatusCode::CONFLICT,
                "account changed while OAuth authorization was in progress; retry the operation"
                    .to_string(),
            ));
        }
        Some(account)
    } else {
        let current_plan_type = normalize_plan_type(input.claims.chatgpt_plan_type.as_deref());
        if let Err((status, message)) = ensure_display_name_available_for_oauth_identity(
            &mut *tx,
            &input.display_name,
            None,
            input.claims.chatgpt_account_id.as_deref(),
            input.claims.chatgpt_user_id.as_deref(),
            session.group_name.as_deref(),
            current_plan_type.as_deref(),
        )
        .await
        {
            if status == StatusCode::CONFLICT {
                fail_login_session_with_executor(&mut *tx, &session.login_id, &message)
                    .await
                    .map_err(internal_error_tuple)?;
                tx.commit().await.map_err(internal_error_tuple)?;
            }
            return Err((status, message));
        }
        None
    };
    let (
        effective_display_name,
        effective_chosen_email,
        effective_group_name,
        effective_is_mother,
        effective_note,
        effective_tag_ids,
        effective_group_metadata_changes,
    ) = if let Some(account) = existing_account {
        (
            account.display_name,
            account.email,
            account.group_name,
            account.is_mother != 0,
            account.note,
            current_account_tag_ids_with_executor(tx.as_mut(), account.id)
                .await
                .map_err(internal_error_tuple)?,
            RequestedGroupMetadataChanges::default(),
        )
    } else {
        (
            input.display_name.clone(),
            input.chosen_email.clone(),
            session.group_name.clone(),
            session.is_mother != 0,
            session.note.clone(),
            parse_tag_ids_json(session.tag_ids_json.as_deref()),
            build_requested_group_metadata_changes(
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
                Some(decode_group_single_account_rotation_enabled(
                    session.group_single_account_rotation_enabled,
                )),
                decode_group_requested_flag(
                    session.group_single_account_rotation_enabled_requested,
                ),
            ),
        )
    };
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: session.account_id,
            display_name: &effective_display_name,
            chosen_email: effective_chosen_email,
            verified_email: input.verified_email.clone(),
            group_name: effective_group_name,
            is_mother: effective_is_mother,
            note: effective_note,
            tag_ids: effective_tag_ids,
            requested_group_metadata_changes: effective_group_metadata_changes,
            claims: &input.claims,
            encrypted_credentials: input.encrypted_credentials,
            has_refresh_token: input.has_refresh_token,
            token_expires_at: &input.token_expires_at,
            external_identity: None,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_refresh_state
        SET lease_token = NULL,
            updated_at = ?3
        WHERE account_id = ?1
          AND operation = ?2
          AND lease_token IS NOT NULL
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS)
    .bind(format_utc_iso(Utc::now()))
    .execute(tx.as_mut())
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
    complete_upstream_account_refresh_success_state_with_executor(
        &mut tx,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_CALLBACK,
        credential_refresh_next_attempt_at.as_deref(),
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Err(err) = record_upstream_account_refresh_success_event(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_CALLBACK,
    )
    .await
    {
        warn!(
            account_id,
            error = %err,
            "OAuth callback committed but credential refresh success history event could not be recorded"
        );
    }
    Ok(account_id)
}

pub(crate) async fn confirm_oauth_identity_overwrite_inner(
    state: &AppState,
    login_id: &str,
) -> Result<i64, (StatusCode, String)> {
    let account_id = load_login_session_by_login_id(&state.pool, login_id)
        .await
        .map_err(internal_error_tuple)?
        .and_then(|session| session.account_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(account_id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id_with_executor(&mut *tx, login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    if session.status != LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION {
        return Err((
            StatusCode::BAD_REQUEST,
            "This login session is not waiting for identity confirmation.".to_string(),
        ));
    }
    let account_id = session.account_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "login session is missing its target account".to_string(),
        )
    })?;
    let account = load_upstream_account_row_conn(tx.as_mut(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    if session
        .account_updated_at
        .as_deref()
        .is_some_and(|expected| expected != account.updated_at)
    {
        return Err((
            StatusCode::CONFLICT,
            "account changed while OAuth authorization was in progress; retry the operation"
                .to_string(),
        ));
    }
    let encrypted_credentials = session
        .pending_encrypted_credentials
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "login session is missing pending credentials".to_string(),
            )
        })?;
    let token_expires_at = session.pending_token_expires_at.clone().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "login session is missing pending token expiry".to_string(),
        )
    })?;
    let credential_refresh_next_attempt_at = parse_rfc3339_utc(&token_expires_at).map(|expires| {
        format_utc_iso(
            expires
                - ChronoDuration::seconds(
                    state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                ),
        )
    });
    let claims = ChatgptJwtClaims {
        email: session.pending_verified_email.clone(),
        chatgpt_plan_type: session.pending_plan_type.clone(),
        chatgpt_user_id: session.pending_chatgpt_user_id.clone(),
        chatgpt_account_id: session.pending_chatgpt_account_id.clone(),
    };
    let tag_ids = current_account_tag_ids_with_executor(tx.as_mut(), account_id)
        .await
        .map_err(internal_error_tuple)?;
    let resolved_account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(account_id),
            display_name: &account.display_name,
            chosen_email: account.email.clone(),
            verified_email: session.pending_verified_email.clone(),
            group_name: account.group_name.clone(),
            is_mother: account.is_mother != 0,
            note: account.note.clone(),
            tag_ids,
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &claims,
            encrypted_credentials,
            has_refresh_token: session.pending_has_refresh_token.unwrap_or(0) != 0,
            token_expires_at: &token_expires_at,
            external_identity: None,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_refresh_state
        SET lease_token = NULL,
            updated_at = ?3
        WHERE account_id = ?1
          AND operation = ?2
          AND lease_token IS NOT NULL
        "#,
    )
    .bind(resolved_account_id)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS)
    .bind(format_utc_iso(Utc::now()))
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;
    let completed_group_metadata_snapshot = load_group_metadata_snapshot_conn_with_limit(
        tx.as_mut(),
        account.group_name.as_deref(),
        session.group_note.as_deref(),
        session.group_concurrency_limit,
    )
    .await
    .map_err(internal_error_tuple)?;
    complete_login_session_with_executor(
        &mut *tx,
        &session.login_id,
        resolved_account_id,
        completed_group_metadata_snapshot.note,
        completed_group_metadata_snapshot.concurrency_limit,
        &session.updated_at,
        false,
    )
    .await
    .map_err(internal_error_tuple)?;
    complete_upstream_account_refresh_success_state_with_executor(
        &mut tx,
        resolved_account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_CALLBACK,
        credential_refresh_next_attempt_at.as_deref(),
    )
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Err(err) = record_upstream_account_refresh_success_event(
        &state.pool,
        resolved_account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_CALLBACK,
    )
    .await
    {
        warn!(
            account_id = resolved_account_id,
            error = %err,
            "OAuth identity overwrite committed but credential refresh success history event could not be recorded"
        );
    }
    drop(_credential_refresh_guard);
    if let Err(err) = sync_upstream_account_by_id(state, account_id, SyncCause::PostCreate).await {
        warn!(account_id, error = %err, "OAuth identity overwrite confirmed but initial sync failed");
    }
    if let Err(err) =
        publish_account_effective_routing_rules_changed(state, Some(&[account_id]), &[]).await
    {
        warn!(
            ?err,
            account_id, "OAuth identity overwrite committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
    Ok(account_id)
}

fn oauth_identity_requires_confirmation(
    account: &UpstreamAccountRow,
    input: &PersistOauthCallbackInput,
) -> bool {
    let Some(existing) = comparable_oauth_identity(
        account.chatgpt_account_id.as_deref(),
        account.chatgpt_user_id.as_deref(),
        account
            .verified_email
            .as_deref()
            .or(account.email.as_deref()),
    ) else {
        return false;
    };
    let Some(incoming) = comparable_oauth_identity(
        input.claims.chatgpt_account_id.as_deref(),
        input.claims.chatgpt_user_id.as_deref(),
        input
            .verified_email
            .as_deref()
            .or(input.chosen_email.as_deref()),
    ) else {
        return false;
    };
    existing != incoming
}

#[derive(Debug, PartialEq, Eq)]
enum ComparableOauthIdentity {
    AccountAndUser { account_id: String, user_id: String },
    AccountAndEmail { account_id: String, email: String },
}

fn comparable_oauth_identity(
    chatgpt_account_id: Option<&str>,
    chatgpt_user_id: Option<&str>,
    email: Option<&str>,
) -> Option<ComparableOauthIdentity> {
    let account_id = normalize_display_name_or_email_key(chatgpt_account_id)?;
    if let Some(user_id) = normalize_display_name_or_email_key(chatgpt_user_id) {
        return Some(ComparableOauthIdentity::AccountAndUser {
            account_id,
            user_id,
        });
    }
    normalize_email_value(email.map(str::to_string))
        .map(|email| ComparableOauthIdentity::AccountAndEmail { account_id, email })
}

async fn mark_login_session_needs_identity_confirmation_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    session: &OauthLoginSessionRow,
    input: &PersistOauthCallbackInput,
) -> Result<()> {
    let updated_at = next_login_session_updated_at(Some(&session.updated_at));
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            error_message = ?3,
            pending_encrypted_credentials = ?4,
            pending_token_expires_at = ?5,
            pending_verified_email = ?6,
            pending_chatgpt_account_id = ?7,
            pending_chatgpt_user_id = ?8,
            pending_plan_type = ?9,
            pending_has_refresh_token = ?10,
            updated_at = ?11,
            consumed_at = NULL
        WHERE login_id = ?1
        "#,
    )
    .bind(&session.login_id)
    .bind(LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION)
    .bind("OAuth identity differs from the existing account. Confirm overwrite to continue.")
    .bind(&input.encrypted_credentials)
    .bind(&input.token_expires_at)
    .bind(&input.verified_email)
    .bind(&input.claims.chatgpt_account_id)
    .bind(&input.claims.chatgpt_user_id)
    .bind(&input.claims.chatgpt_plan_type)
    .bind(if input.has_refresh_token {
        1_i64
    } else {
        0_i64
    })
    .bind(&updated_at)
    .execute(executor)
    .await?;
    Ok(())
}

async fn current_account_tag_ids_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    account_id: i64,
) -> Result<Vec<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT tag_id
        FROM pool_upstream_account_tags
        WHERE account_id = ?1
        ORDER BY tag_id ASC
        "#,
    )
    .bind(account_id)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
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
