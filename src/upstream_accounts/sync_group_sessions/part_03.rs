pub(crate) async fn prepare_pool_account_with_node_shunt_refresh(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: &UpstreamAccountGroupMetadata,
    node_shunt_assignments: &mut UpstreamAccountNodeShuntAssignments,
) -> Result<Option<PoolResolvedAccount>> {
    let mut prepared_account = prepare_pool_account(
        state,
        row,
        effective_rule,
        group_metadata.clone(),
        node_shunt_assignments,
        None,
    )
    .await;
    if group_metadata.node_shunt_enabled
        && prepared_account
            .as_ref()
            .err()
            .is_some_and(|err| is_group_node_shunt_unassigned_message(&err.to_string()))
    {
        *node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
        prepared_account = prepare_pool_account(
            state,
            row,
            effective_rule,
            group_metadata.clone(),
            node_shunt_assignments,
            None,
        )
        .await;
    }
    if group_metadata.node_shunt_enabled && matches!(prepared_account, Ok(None)) {
        *node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    }
    prepared_account
}

pub(crate) fn resolve_account_forward_proxy_scope_from_assignments(
    account_id: i64,
    group_name: Option<&str>,
    group_metadata: &UpstreamAccountGroupMetadata,
    assignments: &UpstreamAccountNodeShuntAssignments,
) -> Result<ForwardProxyRouteScope> {
    if !group_metadata.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            group_name,
            group_metadata.bound_proxy_keys.clone(),
        );
    }

    let normalized_group_name = group_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let Some(proxy_key) = assignments.account_proxy_keys.get(&account_id) else {
        bail!(group_node_shunt_unassigned_error_message());
    };
    if !assignments.group_slots.contains_key(normalized_group_name) {
        return Err(group_node_shunt_unassigned_error());
    }
    Ok(ForwardProxyRouteScope::pinned(proxy_key.clone()))
}

pub(crate) fn account_bound_forward_proxy_scope(
    row: &UpstreamAccountRow,
) -> Option<ForwardProxyRouteScope> {
    let bound_proxy_keys = row.bound_proxy_keys();
    (!bound_proxy_keys.is_empty()).then(|| {
        ForwardProxyRouteScope::bound_scope(format!("account:{}", row.id()), bound_proxy_keys)
    })
}

pub(crate) fn transit_account_forward_proxy_scope(
    row: &UpstreamAccountRow,
) -> ForwardProxyRouteScope {
    account_bound_forward_proxy_scope(row).unwrap_or_else(|| {
        ForwardProxyRouteScope::bound_scope(
            format!("account:{}", row.id()),
            vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
        )
    })
}

pub(crate) async fn resolve_account_forward_proxy_scope(
    state: &AppState,
    row: &UpstreamAccountRow,
    group_metadata: Option<UpstreamAccountGroupMetadata>,
) -> Result<ForwardProxyRouteScope> {
    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        return Ok(transit_account_forward_proxy_scope(row));
    }
    let group_metadata = match group_metadata {
        Some(metadata) => metadata,
        None => load_group_metadata(&state.pool, row.group_name.as_deref()).await?,
    };
    if let Some(scope) = account_bound_forward_proxy_scope(row) {
        return Ok(scope);
    }
    if !group_metadata.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            row.group_name.as_deref(),
            group_metadata.bound_proxy_keys,
        );
    }
    let assignments = build_upstream_account_node_shunt_assignments(state).await?;
    resolve_account_forward_proxy_scope_from_assignments(
        row.id,
        row.group_name.as_deref(),
        &group_metadata,
        &assignments,
    )
}

pub(crate) async fn resolve_account_forward_proxy_scope_for_sync(
    state: &AppState,
    row: &UpstreamAccountRow,
    group_metadata: Option<UpstreamAccountGroupMetadata>,
) -> Result<ForwardProxyRouteScope> {
    let group_metadata = match group_metadata {
        Some(metadata) => metadata,
        None => load_group_metadata(&state.pool, row.group_name.as_deref()).await?,
    };
    if let Some(scope) = account_bound_forward_proxy_scope(row) {
        return Ok(scope);
    }
    if !group_metadata.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            row.group_name.as_deref(),
            group_metadata.bound_proxy_keys,
        );
    }

    let normalized_group_name = row
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let normalized_bound_proxy_keys =
        normalize_bound_proxy_keys(group_metadata.bound_proxy_keys.clone());
    if normalized_bound_proxy_keys.is_empty() {
        bail!(missing_group_bound_proxy_error_message(
            normalized_group_name
        ));
    }
    let valid_proxy_keys = {
        let manager = state.forward_proxy.lock().await;
        manager.selectable_bound_proxy_keys_in_order(&normalized_bound_proxy_keys)
    };
    if valid_proxy_keys.is_empty() {
        bail!(missing_selectable_group_bound_proxy_error_message(
            normalized_group_name
        ));
    }

    let reservation_snapshot = pool_routing_reservation_snapshot(state);
    if let Some(proxy_key) = reservation_snapshot
        .pinned_proxy_keys_for_account(row.id, &valid_proxy_keys, &HashSet::new())
        .into_iter()
        .next()
    {
        return Ok(ForwardProxyRouteScope::pinned(proxy_key));
    }

    let assignments = build_upstream_account_node_shunt_assignments(state).await?;
    if let Some(proxy_key) = assignments.account_proxy_keys.get(&row.id) {
        return Ok(ForwardProxyRouteScope::pinned(proxy_key.clone()));
    }

    required_account_forward_proxy_scope(Some(normalized_group_name), valid_proxy_keys)
}

pub(crate) async fn resolve_group_forward_proxy_scope_for_provisioning(
    state: &AppState,
    binding: &ResolvedRequiredGroupProxyBinding,
    assignments: Option<&UpstreamAccountNodeShuntAssignments>,
    provisioning_account: Option<&UpstreamAccountRow>,
    consumed_proxy_keys: &HashSet<String>,
) -> Result<ForwardProxyRouteScope> {
    if !binding.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            Some(&binding.group_name),
            binding.bound_proxy_keys.clone(),
        );
    }

    let valid_proxy_keys = {
        let manager = state.forward_proxy.lock().await;
        manager.selectable_bound_proxy_keys_in_order(&binding.bound_proxy_keys)
    };
    let globally_occupied_proxy_keys = assignments
        .map(|value| {
            value
                .group_assigned_proxy_keys
                .values()
                .flat_map(|proxy_keys| proxy_keys.iter().cloned())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let reservation_snapshot = pool_routing_reservation_snapshot(state);

    if let (Some(value), Some(account)) = (assignments, provisioning_account)
        && normalize_optional_text(account.group_name.clone()).as_deref()
            == Some(binding.group_name.as_str())
    {
        if let Some(proxy_key) = value.account_proxy_keys.get(&account.id) {
            return Ok(ForwardProxyRouteScope::pinned(proxy_key.clone()));
        }
        if let Some(proxy_key) = reservation_snapshot
            .pinned_proxy_keys_for_account(
                account.id,
                &valid_proxy_keys,
                &globally_occupied_proxy_keys,
            )
            .into_iter()
            .find(|proxy_key| !consumed_proxy_keys.contains(proxy_key))
        {
            return Ok(ForwardProxyRouteScope::pinned(proxy_key));
        }
    }

    let reserved_proxy_keys = reservation_snapshot.reserved_proxy_keys_for_group(&valid_proxy_keys);
    for proxy_key in valid_proxy_keys {
        if globally_occupied_proxy_keys.contains(&proxy_key)
            || reserved_proxy_keys.contains(&proxy_key)
            || consumed_proxy_keys.contains(&proxy_key)
        {
            continue;
        }
        return Ok(ForwardProxyRouteScope::pinned(proxy_key));
    }

    Err(group_node_shunt_unassigned_error())
}

pub(crate) fn reserve_imported_oauth_node_shunt_scope(
    state: &AppState,
    source_id: &str,
    account_id: Option<i64>,
    scope: &ForwardProxyRouteScope,
) -> Result<Option<String>> {
    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        return Ok(None);
    };
    let reservation_key = format!(
        "imported-oauth:{source_id}:{}",
        random_hex(8).map_err(|(_, message)| anyhow!(message))?
    );
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            reservation_key.clone(),
            crate::PoolRoutingReservation {
                account_id: account_id.unwrap_or_default(),
                model: None,
                proxy_key: Some(proxy_key.clone()),
                created_at: Instant::now(),
            },
        );
    Ok(Some(reservation_key))
}

pub(crate) fn release_imported_oauth_node_shunt_scope(
    state: &AppState,
    reservation_key: Option<String>,
) {
    if let Some(reservation_key) = reservation_key {
        crate::release_pool_routing_reservation(state, &reservation_key);
    }
}

pub(crate) async fn load_login_session_by_login_id_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, email, group_name, group_bound_proxy_keys_json, group_node_shunt_enabled,
            group_node_shunt_enabled_requested, group_single_account_rotation_enabled,
            group_single_account_rotation_enabled_requested, is_mother, note, tag_ids_json, group_note,
            group_concurrency_limit,
            mailbox_session_id, generated_mailbox_address AS mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, pending_encrypted_credentials, pending_token_expires_at, pending_verified_email,
            pending_chatgpt_account_id, pending_chatgpt_user_id, pending_plan_type, pending_has_refresh_token,
            expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE login_id = ?1
        LIMIT 1
        "#,
    )
    .bind(login_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_login_session_by_login_id(
    pool: &Pool<Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    load_login_session_by_login_id_with_executor(pool, login_id).await
}

pub(crate) async fn load_login_session_by_state(
    pool: &Pool<Sqlite>,
    state_value: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, email, group_name, group_bound_proxy_keys_json, group_node_shunt_enabled,
            group_node_shunt_enabled_requested, group_single_account_rotation_enabled,
            group_single_account_rotation_enabled_requested, is_mother, note, tag_ids_json, group_note,
            group_concurrency_limit,
            mailbox_session_id, generated_mailbox_address AS mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, pending_encrypted_credentials, pending_token_expires_at, pending_verified_email,
            pending_chatgpt_account_id, pending_chatgpt_user_id, pending_plan_type, pending_has_refresh_token,
            expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE state = ?1
        LIMIT 1
        "#,
    )
    .bind(state_value)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn expire_pending_login_sessions(pool: &Pool<Sqlite>) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?1,
            updated_at = ?2,
            pending_encrypted_credentials = NULL,
            pending_token_expires_at = NULL,
            pending_verified_email = NULL,
            pending_chatgpt_account_id = NULL,
            pending_chatgpt_user_id = NULL,
            pending_plan_type = NULL,
            pending_has_refresh_token = NULL
        WHERE status IN (?3, ?4) AND expires_at < ?2
        "#,
    )
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .bind(LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn load_oauth_mailbox_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Option<OauthMailboxSessionRow>> {
    sqlx::query_as::<_, OauthMailboxSessionRow>(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE session_id = ?1
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_oauth_mailbox_sessions(
    pool: &Pool<Sqlite>,
    session_ids: &[String],
) -> Result<Vec<OauthMailboxSessionRow>> {
    if session_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE session_id IN (
        "#,
    );
    let mut separated = builder.separated(", ");
    for session_id in session_ids {
        separated.push_bind(session_id);
    }
    separated.push_unseparated(")");
    builder.push(" ORDER BY created_at ASC");
    builder
        .build_query_as::<OauthMailboxSessionRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn delete_oauth_mailbox_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    session_id: &str,
) -> Result<u64> {
    let affected = sqlx::query(
        r#"
        DELETE FROM pool_oauth_mailbox_sessions
        WHERE session_id = ?1
        "#,
    )
    .bind(session_id)
    .execute(executor)
    .await?
    .rows_affected();
    Ok(affected)
}

pub(crate) async fn cleanup_expired_oauth_mailbox_sessions(state: &AppState) -> Result<()> {
    let kaisoumail_meta = state.config.upstream_accounts_kaisoumail.as_ref();
    let now_iso = format_utc_iso(Utc::now());
    let expired_rows = sqlx::query_as::<_, OauthMailboxSessionRow>(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE expires_at <= ?1
        ORDER BY expires_at ASC
        "#,
    )
    .bind(&now_iso)
    .fetch_all(&state.pool)
    .await?;

    for row in expired_rows {
        if expired_mailbox_session_requires_remote_delete(&row)
            && let Some(config) = kaisoumail_meta
            && let Err(err) =
                kaisoumail_delete_mailbox(&state.http_clients.shared, config, &row.remote_email_id)
                    .await
        {
            debug!(
                mailbox_session_id = %row.session_id,
                remote_email_id = %row.remote_email_id,
                error = %err,
                "failed to delete expired kaisoumail mailbox"
            );
        }
        delete_oauth_mailbox_session_with_executor(&state.pool, &row.session_id).await?;
    }
    Ok(())
}

pub(crate) async fn complete_login_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
    account_id: i64,
    group_note_snapshot: Option<String>,
    group_concurrency_limit_snapshot: i64,
    previous_updated_at: &str,
    preserve_pending_updated_at: bool,
) -> Result<()> {
    let consumed_at = next_login_session_updated_at(Some(previous_updated_at));
    let completed_updated_at = if preserve_pending_updated_at {
        previous_updated_at.to_string()
    } else {
        consumed_at.clone()
    };
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            account_id = ?3,
            group_note = ?4,
            group_concurrency_limit = ?5,
            updated_at = ?6,
            consumed_at = ?7,
            pending_encrypted_credentials = NULL,
            pending_token_expires_at = NULL,
            pending_verified_email = NULL,
            pending_chatgpt_account_id = NULL,
            pending_chatgpt_user_id = NULL,
            pending_plan_type = NULL,
            pending_has_refresh_token = NULL
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_COMPLETED)
    .bind(account_id)
    .bind(group_note_snapshot)
    .bind(group_concurrency_limit_snapshot)
    .bind(&completed_updated_at)
    .bind(&consumed_at)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn load_group_metadata_snapshot_conn(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: Option<&str>,
    fallback_note: Option<&str>,
) -> Result<UpstreamAccountGroupMetadata> {
    load_group_metadata_snapshot_conn_with_limit(executor, group_name, fallback_note, 0).await
}

pub(crate) async fn load_group_metadata_snapshot_conn_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: Option<&str>,
    fallback_note: Option<&str>,
    fallback_concurrency_limit: i64,
) -> Result<UpstreamAccountGroupMetadata> {
    let normalized_fallback_concurrency_limit =
        normalize_concurrency_limit(Some(fallback_concurrency_limit), "concurrencyLimit")
            .map_err(|(_, message)| anyhow!(message))?;
    let Some(group_name) = group_name else {
        return Ok(UpstreamAccountGroupMetadata {
            note: fallback_note.map(str::to_string),
            bound_proxy_keys: Vec::new(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: normalized_fallback_concurrency_limit,
        });
    };
    let metadata = load_group_metadata_conn(executor, group_name)
        .await?
        .unwrap_or_default();
    Ok(UpstreamAccountGroupMetadata {
        note: metadata
            .note
            .or_else(|| normalize_optional_text(fallback_note.map(str::to_string))),
        bound_proxy_keys: metadata.bound_proxy_keys,
        node_shunt_enabled: metadata.node_shunt_enabled,
        single_account_rotation_enabled: metadata.single_account_rotation_enabled,
        upstream_429_retry_enabled: metadata.upstream_429_retry_enabled,
        upstream_429_max_retries: metadata.upstream_429_max_retries,
        concurrency_limit: metadata.concurrency_limit,
    })
}

pub(crate) fn next_login_session_updated_at(previous_updated_at: Option<&str>) -> String {
    let mut next_updated_at =
        parse_rfc3339_utc(&Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(Utc::now);
    if let Some(previous_updated_at) = previous_updated_at
        && let Some(previous_updated_at) = parse_rfc3339_utc(previous_updated_at)
        && next_updated_at <= previous_updated_at
    {
        next_updated_at = previous_updated_at + ChronoDuration::milliseconds(1);
    }
    next_updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) async fn fail_login_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            error_message = ?3,
            consumed_at = ?4,
            updated_at = ?4,
            pending_encrypted_credentials = NULL,
            pending_token_expires_at = NULL,
            pending_verified_email = NULL,
            pending_chatgpt_account_id = NULL,
            pending_chatgpt_user_id = NULL,
            pending_plan_type = NULL,
            pending_has_refresh_token = NULL
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_FAILED)
    .bind(error_message)
    .bind(&now_iso)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn fail_login_session(
    pool: &Pool<Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    fail_login_session_with_executor(pool, login_id, error_message).await
}

pub(crate) async fn mark_login_session_expired(pool: &Pool<Sqlite>, login_id: &str) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            updated_at = ?3
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn login_session_to_response(row: &OauthLoginSessionRow) -> LoginSessionStatusResponse {
    LoginSessionStatusResponse {
        login_id: row.login_id.clone(),
        status: row.status.clone(),
        auth_url: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.auth_url.clone())
        } else {
            None
        },
        redirect_uri: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.redirect_uri.clone())
        } else {
            None
        },
        expires_at: row.expires_at.clone(),
        updated_at: row.updated_at.clone(),
        account_id: row.account_id,
        email: row.email.clone(),
        error: row.error_message.clone(),
        sync_applied: None,
        identity_confirmation: login_session_identity_confirmation_response(row),
    }
}

pub(crate) fn login_session_identity_confirmation_response(
    row: &OauthLoginSessionRow,
) -> Option<OauthIdentityConfirmationResponse> {
    if row.status != LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION {
        return None;
    }
    Some(OauthIdentityConfirmationResponse {
        current: OauthIdentitySummaryResponse {
            account_id: row.account_id,
            display_name: row.display_name.clone(),
            email: row.email.clone(),
            verified_email: None,
            chatgpt_account_id: None,
            chatgpt_user_id: None,
            plan_type: None,
        },
        incoming: OauthIdentitySummaryResponse {
            account_id: row.account_id,
            display_name: None,
            email: row.pending_verified_email.clone(),
            verified_email: row.pending_verified_email.clone(),
            chatgpt_account_id: row.pending_chatgpt_account_id.clone(),
            chatgpt_user_id: row.pending_chatgpt_user_id.clone(),
            plan_type: row.pending_plan_type.clone(),
        },
    })
}

pub(crate) fn login_session_to_response_with_sync_applied(
    row: &OauthLoginSessionRow,
    sync_applied: bool,
) -> LoginSessionStatusResponse {
    let mut response = login_session_to_response(row);
    response.sync_applied = Some(sync_applied);
    response
}

pub(crate) fn login_session_required_forward_proxy_scope(
    row: &OauthLoginSessionRow,
) -> Result<ForwardProxyRouteScope> {
    required_account_forward_proxy_scope(
        row.group_name.as_deref(),
        decode_group_bound_proxy_keys_json(row.group_bound_proxy_keys_json.as_deref()),
    )
}
