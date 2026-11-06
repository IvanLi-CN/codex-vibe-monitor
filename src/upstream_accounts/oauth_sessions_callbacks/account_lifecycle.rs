use super::*;
use sqlx::Transaction;

pub(crate) async fn relogin_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    let payload = CreateOauthLoginSessionRequest {
        display_name: None,
        email: None,
        group_name: None,
        group_bound_proxy_keys: None,
        group_node_shunt_enabled: None,
        group_single_account_rotation_enabled: None,
        note: None,
        group_note: None,
        concurrency_limit: None,
        account_id: Some(id),
        tag_ids: Vec::new(),
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
    if payload.group_name.is_some()
        || payload.group_bound_proxy_keys.is_some()
        || payload.group_node_shunt_enabled.is_some()
        || payload.group_single_account_rotation_enabled.is_some()
        || payload.group_note.is_some()
        || payload.concurrency_limit.is_some()
        || payload.is_mother == Some(true)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "API Key accounts cannot use groups, group policies, or mother-account routing"
                .to_string(),
        ));
    }
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let display_name = normalize_required_display_name(&payload.display_name)?;
    validate_local_limits(payload.local_primary_limit, payload.local_secondary_limit)?;
    let api_key = normalize_required_secret(&payload.api_key, "apiKey")?;
    let email = normalize_optional_email(payload.email, "email")?;
    reject_manual_tag_ids(&payload.tag_ids)?;
    let group_name: Option<String> = None;
    let note = normalize_optional_text(payload.note);
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        None, false, None, false, 0, false, None, false, None, false,
    );
    let target_group_name: Option<String> = None;
    let is_mother = false;
    let bound_proxy_keys = match payload.bound_proxy_keys {
        OptionalField::Missing => vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
        OptionalField::Null => {
            return Err((
                StatusCode::BAD_REQUEST,
                "API Key accounts require at least one bound proxy node".to_string(),
            ));
        }
        OptionalField::Value(values) => {
            let normalized = normalize_bound_proxy_keys(values);
            if normalized.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "API Key accounts require at least one bound proxy node".to_string(),
                ));
            }
            let canonical = canonicalize_forward_proxy_bound_keys(state.as_ref(), &normalized)
                .await
                .map_err(internal_error_tuple)?;
            let has_selectable = {
                let manager = state.forward_proxy.lock().await;
                manager.has_selectable_bound_proxy_keys(&canonical)
            };
            if !has_selectable {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "API Key accounts require at least one available proxy node".to_string(),
                ));
            }
            canonical
        }
    };
    let bound_proxy_keys_json =
        encode_group_bound_proxy_keys_json(&bound_proxy_keys).map_err(internal_error_tuple)?;
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
            kind, provider, display_name, group_name, is_mother, note, status, enabled, email, verified_email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key, encrypted_credentials, token_expires_at,
            last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
            local_primary_limit, local_secondary_limit, local_limit_unit, upstream_base_url,
            bound_proxy_keys_json, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, NULL,
            NULL, NULL, NULL, NULL, ?9, ?10, NULL,
            NULL, NULL, NULL, NULL, NULL,
            ?11, ?12, ?13, ?14, ?15, ?16, ?16
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
    .bind(email)
    .bind(masked_api_key)
    .bind(encrypted_credentials)
    .bind(payload.local_primary_limit)
    .bind(payload.local_secondary_limit)
    .bind(limit_unit)
    .bind(upstream_base_url)
    .bind(bound_proxy_keys_json)
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

    // API-key relays are account-scoped and do not participate in OAuth maintenance sync.
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), inserted_id)
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
        .and_then(|detail| {
            detail.ok_or_else(|| anyhow::anyhow!("created API Key account was not found"))
        });
    let routing_scope = if requested_group_metadata_changes.was_requested() {
        None
    } else {
        Some(std::slice::from_ref(&inserted_id))
    };
    let routing_state_version =
        match publish_account_effective_routing_rules_changed(state.as_ref(), routing_scope, &[])
            .await
        {
            Ok(version) => Some(version),
            Err(err) => {
                warn!(
                    ?err,
                    account_id = inserted_id,
                    "account create committed but routing publication failed"
                );
                invalidate_dashboard_activity_snapshots_with_accounts(
                    state.dashboard_activity_snapshot_cache.as_ref(),
                    "account_effective_routing_rules_publication_failed",
                )
                .await;
                None
            }
        };
    publish_new_account_routing_availability_if_selectable(state.as_ref(), inserted_id).await;
    let mut detail = detail.map_err(request_runtime_error_tuple)?;
    detail.routing_state_version = routing_state_version;
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

pub(crate) async fn update_upstream_account_model_mappings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateModelMappingsRequest>,
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
        .run_update_model_mappings(state.clone(), id, payload)
        .await?;
    Ok(Json(detail))
}

pub(crate) async fn update_upstream_account_model_mappings_inner(
    state: &AppState,
    id: i64,
    payload: UpdateModelMappingsRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let mappings = normalize_model_mappings(payload.model_mappings)?;
    let mappings_json = encode_model_mappings_json(&mappings).map_err(internal_error_tuple)?;
    // Initialize lazily before taking the replacement lock. The initializer itself
    // builds a full cache and takes that same lock when no cache exists yet.
    load_pool_routing_runtime_cache(state)
        .await
        .map_err(internal_error_tuple)?;
    let _model_cache_write_guard = state.pool_model_routing_cache_write_lock.lock().await;
    let runtime_cache = state
        .pool_routing_runtime_cache
        .lock()
        .await
        .as_ref()
        .cloned()
        .expect("pool routing runtime cache should be initialized before mapping save");
    let model_routing = build_pool_model_routing_runtime_cache_with_mapping_override(
        &state.pool,
        Some((id, &mappings)),
    )
    .await
    .map_err(internal_error_tuple)?;
    let now_iso = format_utc_iso(Utc::now());
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_accounts WHERE id = ?1 AND COALESCE(deleted_at, '') = ''",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    if exists == 0 {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET model_mappings_json = ?2,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(mappings_json)
    .bind(now_iso)
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    sqlx::query("DELETE FROM pool_upstream_account_model_routes WHERE account_id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal_error_tuple)?;
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_tags
        WHERE account_id = ?1
          AND tag_id IN (
              SELECT id FROM pool_tags WHERE system_key LIKE 'unsupported_model:%'
          )
        "#,
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;

    install_pool_model_routing_runtime_cache(state, runtime_cache, model_routing).await;
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
    publish_pool_routing_availability(state);
    load_upstream_account_detail_with_actual_usage(state, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))
}

pub(crate) async fn apply_oauth_login_session_metadata_to_account_with_executor(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: i64,
    display_name: Option<String>,
    email: Option<String>,
    group_name: Option<String>,
    note: Option<String>,
    requested_group_metadata_changes: &RequestedGroupMetadataChanges,
    is_mother: bool,
) -> Result<(), (StatusCode, String)> {
    let row = load_upstream_account_row_conn(tx.as_mut(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let previous_group_name = row.group_name.clone();
    let next_display_name = resolve_display_name_after_email_change(
        Some(row.display_name.as_str()),
        display_name.as_deref(),
        row.email.as_deref(),
        email.as_deref().or(row.email.as_deref()),
    )
    .unwrap_or(row.display_name.clone());
    let effective_plan_type =
        resolve_effective_plan_type_for_account(tx.as_mut(), account_id, row.plan_type.as_deref())
            .await
            .map_err(internal_error_tuple)?;
    ensure_display_name_available_for_oauth_identity(
        tx.as_mut(),
        &next_display_name,
        Some(account_id),
        row.chatgpt_account_id.as_deref(),
        row.chatgpt_user_id.as_deref(),
        group_name.as_deref().or(row.group_name.as_deref()),
        effective_plan_type.as_deref(),
    )
    .await?;

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            email = ?3,
            group_name = ?4,
            is_mother = ?5,
            note = ?6,
            updated_at = ?7
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(&next_display_name)
    .bind(email.or(row.email))
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
    let account_row = load_upstream_account_row_conn(tx.as_mut(), id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    let account_group_name = account_row.group_name.clone();
    if account_row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET enabled = 0,
                status = ?1,
                encrypted_credentials = NULL,
                masked_api_key = NULL,
                token_expires_at = NULL,
                last_error = NULL,
                last_action = 'deleted',
                last_action_source = 'settings',
                last_activity_live_backfill_completed = 1,
                last_activity_archive_backfill_completed = 1,
                deleted_at = datetime('now'),
                updated_at = datetime('now')
            WHERE id = ?2
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_STATUS_DISABLED)
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        sqlx::query("DELETE FROM pool_oauth_login_sessions WHERE account_id = ?1")
            .bind(id)
            .execute(tx.as_mut())
            .await
            .map_err(internal_error_tuple)?;
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
        let sticky_clear_now = format_utc_iso_precise(Utc::now());
        delete_sticky_routes_for_account_executor(tx.as_mut(), id, &sticky_clear_now)
            .await
            .map_err(internal_error_tuple)?;
        sqlx::query("DELETE FROM pool_upstream_account_model_routes WHERE account_id = ?1")
            .bind(id)
            .execute(tx.as_mut())
            .await
            .map_err(internal_error_tuple)?;
        sqlx::query(
            "DELETE FROM prompt_cache_conversation_bindings WHERE upstream_account_id = ?1",
        )
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        sqlx::query(
            "DELETE FROM prompt_cache_encrypted_session_owners WHERE owner_upstream_account_id = ?1",
        )
        .bind(id)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        cleanup_orphaned_group_metadata(tx.as_mut(), account_group_name.as_deref())
            .await
            .map_err(internal_error_tuple)?;
        tx.commit().await.map_err(internal_error_tuple)?;
        if let Ok(mut selected_at) = state.pool_account_selection_runtime.selected_at.lock() {
            selected_at.remove(&id);
        }
        if let Err(err) =
            publish_account_effective_routing_rules_changed(state, Some(&[]), &[id]).await
        {
            warn!(
                ?err,
                account_id = id,
                "API key account delete committed but routing publication failed"
            );
            invalidate_dashboard_activity_snapshots_with_accounts(
                state.dashboard_activity_snapshot_cache.as_ref(),
                "account_effective_routing_rules_publication_failed",
            )
            .await;
        }
        return Ok(StatusCode::NO_CONTENT);
    }
    let group_name = account_row.group_name;
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
    cleanup_orphaned_group_metadata(tx.as_mut(), group_name.as_deref())
        .await
        .map_err(internal_error_tuple)?;
    tx.commit().await.map_err(internal_error_tuple)?;
    if let Err(err) = publish_account_effective_routing_rules_changed(state, Some(&[]), &[id]).await
    {
        warn!(
            ?err,
            account_id = id,
            "OAuth account delete committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
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

fn manual_refresh_response_status(detail: &UpstreamAccountDetail, operation: &str) -> StatusCode {
    if operation == UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS
        && (detail.credential_refresh.state == UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH
            || detail.summary.status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    {
        return StatusCode::CONFLICT;
    }
    let state = match operation {
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS => detail.credential_refresh.state.as_str(),
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS => {
            detail.usage_window_refresh.state.as_str()
        }
        _ => UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED,
    };
    match state {
        UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH => StatusCode::CONFLICT,
        UPSTREAM_ACCOUNT_REFRESH_STATE_PARTIAL
            if operation == UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS =>
        {
            StatusCode::MULTI_STATUS
        }
        UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING
        | UPSTREAM_ACCOUNT_REFRESH_STATE_ERROR
        | UPSTREAM_ACCOUNT_REFRESH_STATE_REFRESHING => StatusCode::BAD_GATEWAY,
        _ => StatusCode::OK,
    }
}

fn manual_refresh_response(detail: UpstreamAccountDetail, operation: &str) -> Response {
    (
        manual_refresh_response_status(&detail, operation),
        Json(detail),
    )
        .into_response()
}

fn manual_refresh_request_error_tuple(err: impl ToString) -> (StatusCode, String) {
    let message = err.to_string();
    match message.as_str() {
        "account not found" => (StatusCode::NOT_FOUND, message),
        "usage window refresh only supports OAuth accounts"
        | "credential refresh only supports OAuth accounts"
        | "disabled accounts cannot refresh usage windows"
        | "disabled accounts cannot refresh credentials" => (StatusCode::BAD_REQUEST, message),
        _ => request_runtime_error_tuple(message),
    }
}

pub(crate) async fn refresh_upstream_account_usage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Response, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let detail = state
        .upstream_accounts
        .account_ops
        .run_manual_usage_refresh(state.clone(), id)
        .await
        .map_err(manual_refresh_request_error_tuple)?;
    Ok(manual_refresh_response(
        detail,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    ))
}

pub(crate) async fn refresh_upstream_account_credentials(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Response, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let detail = state
        .upstream_accounts
        .account_ops
        .run_manual_credential_refresh(state.clone(), id)
        .await
        .map_err(manual_refresh_request_error_tuple)?;
    Ok(manual_refresh_response(
        detail,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    ))
}
