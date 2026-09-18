use super::*;
use sqlx::Transaction;

struct PreparedExternalOauthUpsert {
    identity: ExternalAccountIdentity,
    metadata: ExternalUpstreamAccountMetadataRequest,
    normalized: NormalizedImportedOauthCredentials,
    probe: ImportedOauthProbeOutcome,
    existing_account_id: Option<i64>,
}

struct ExternalExistingOauthUpsertPlan {
    display_name: String,
    chosen_email: Option<String>,
    verified_email: Option<String>,
    group_name: Option<String>,
    is_mother: bool,
    note: Option<String>,
    tag_ids: Vec<i64>,
    requested_group_metadata_changes: RequestedGroupMetadataChanges,
    encrypted_credentials: String,
    next_enabled: bool,
    routing_scope_is_global: bool,
}

struct ExternalOauthCreatePlan {
    group_name: Option<String>,
    is_mother: bool,
    note: Option<String>,
    tag_ids: Vec<i64>,
    requested_group_metadata_changes: RequestedGroupMetadataChanges,
    encrypted_credentials: String,
    next_enabled: bool,
}

enum ExternalOauthPersistenceResult {
    Created(i64),
    Existing(UpstreamAccountDetail),
}

fn normalize_external_source_account_id(raw: &str) -> Result<String, (StatusCode, String)> {
    let Some(value) = normalize_optional_text(Some(raw.to_string())) else {
        return Err((
            StatusCode::BAD_REQUEST,
            "sourceAccountId must be a non-empty string".to_string(),
        ));
    };
    if value.len() > 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            "sourceAccountId must not exceed 128 characters".to_string(),
        ));
    }
    Ok(value)
}

fn short_external_identity_suffix(raw: &str) -> String {
    let mut suffix = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    suffix = suffix.trim_matches('-').to_string();
    if suffix.is_empty() {
        suffix = "external".to_string();
    }
    suffix.chars().take(24).collect()
}

fn short_external_identity_hash(client_id: &str, source_account_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(client_id.as_bytes());
    hasher.update(b":");
    hasher.update(source_account_id.as_bytes());
    let digest = hasher.finalize();
    let mut output = String::with_capacity(8);
    for byte in digest.iter().take(4) {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

async fn resolve_external_create_display_name(
    tx: &mut Transaction<'_, Sqlite>,
    identity: &ExternalAccountIdentity,
    metadata: &ExternalUpstreamAccountMetadataRequest,
    normalized: &NormalizedImportedOauthCredentials,
) -> Result<String, (StatusCode, String)> {
    if let Some(display_name) = metadata.display_name.as_deref() {
        let normalized_display_name = normalize_required_display_name(display_name)?;
        ensure_display_name_available(&mut **tx, &normalized_display_name, None).await?;
        return Ok(normalized_display_name);
    }

    let direct_candidate = normalize_required_display_name(&normalized.display_name)?;
    if load_conflicting_display_name_id(&mut **tx, &direct_candidate, None)
        .await
        .map_err(internal_error_tuple)?
        .is_none()
    {
        return Ok(direct_candidate);
    }

    let suffixed_candidate = normalize_required_display_name(&format!(
        "{} [{}]",
        normalized.display_name,
        short_external_identity_suffix(&identity.source_account_id)
    ))?;
    if load_conflicting_display_name_id(&mut **tx, &suffixed_candidate, None)
        .await
        .map_err(internal_error_tuple)?
        .is_none()
    {
        return Ok(suffixed_candidate);
    }

    let hashed_candidate = normalize_required_display_name(&format!(
        "{} [{}]",
        normalized.display_name,
        short_external_identity_hash(&identity.client_id, &identity.source_account_id)
    ))?;
    ensure_display_name_available(&mut **tx, &hashed_candidate, None).await?;
    Ok(hashed_candidate)
}

fn normalize_external_group_name(
    metadata: &ExternalUpstreamAccountMetadataRequest,
    existing_row: Option<&UpstreamAccountRow>,
) -> Option<String> {
    let group_name = if metadata.group_name.is_some() {
        metadata.group_name.clone()
    } else {
        existing_row.and_then(|row| row.group_name.clone())
    };
    Some(normalize_upstream_account_group_name(group_name))
}

pub(crate) fn external_group_binding_name(
    metadata: &ExternalUpstreamAccountMetadataRequest,
    existing_row: Option<&UpstreamAccountRow>,
) -> Option<String> {
    let normalized = normalize_external_group_name(metadata, existing_row)?;
    let proxy_metadata_changed = metadata.group_bound_proxy_keys.is_some()
        || metadata.group_node_shunt_enabled.is_some()
        || metadata.group_single_account_rotation_enabled.is_some();
    if normalized != DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME
        || metadata.group_name.is_some()
        || proxy_metadata_changed
    {
        return Some(normalized);
    }
    None
}

async fn load_external_account_tag_ids(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Vec<i64>, (StatusCode, String)> {
    Ok(load_account_tag_map(pool, &[account_id])
        .await
        .map_err(internal_error_tuple)?
        .remove(&account_id)
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.id)
        .collect())
}

async fn resolve_external_group_binding(
    state: &AppState,
    metadata: &ExternalUpstreamAccountMetadataRequest,
    existing_row: Option<&UpstreamAccountRow>,
) -> Result<Option<ResolvedRequiredGroupProxyBinding>, (StatusCode, String)> {
    let target_group_name = external_group_binding_name(metadata, existing_row);

    if target_group_name.is_some()
        && (metadata.group_name.is_some()
            || metadata.group_bound_proxy_keys.is_some()
            || metadata.group_node_shunt_enabled.is_some()
            || metadata.group_single_account_rotation_enabled.is_some())
    {
        return resolve_required_group_proxy_binding_for_write(
            state,
            target_group_name,
            metadata.group_bound_proxy_keys.clone(),
            metadata.group_node_shunt_enabled,
        )
        .await
        .map(Some);
    }

    let Some(group_name) = target_group_name else {
        return Ok(None);
    };
    let existing_group_metadata = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    Ok(Some(ResolvedRequiredGroupProxyBinding {
        group_name,
        bound_proxy_keys: existing_group_metadata.bound_proxy_keys,
        node_shunt_enabled: existing_group_metadata.node_shunt_enabled,
    }))
}

fn external_metadata_to_update_request(
    metadata: ExternalUpstreamAccountMetadataRequest,
) -> UpdateUpstreamAccountRequest {
    UpdateUpstreamAccountRequest {
        display_name: metadata.display_name,
        email: OptionalField::Missing,
        group_name: metadata.group_name,
        group_bound_proxy_keys: metadata.group_bound_proxy_keys,
        group_node_shunt_enabled: metadata.group_node_shunt_enabled,
        group_single_account_rotation_enabled: metadata.group_single_account_rotation_enabled,
        note: metadata.note,
        group_note: metadata.group_note,
        concurrency_limit: metadata.concurrency_limit,
        upstream_base_url: OptionalField::Missing,
        bound_proxy_keys: OptionalField::Missing,
        enabled: metadata.enabled,
        is_mother: metadata.is_mother,
        api_key: None,
        local_primary_limit: None,
        local_secondary_limit: None,
        local_limit_unit: None,
        tag_ids: metadata.tag_ids,
        routing_rule: None,
        ..UpdateUpstreamAccountRequest::default()
    }
}

fn external_metadata_has_any_change(metadata: &ExternalUpstreamAccountMetadataRequest) -> bool {
    metadata.display_name.is_some()
        || metadata.group_name.is_some()
        || metadata.group_bound_proxy_keys.is_some()
        || metadata.group_node_shunt_enabled.is_some()
        || metadata.group_single_account_rotation_enabled.is_some()
        || metadata.note.is_some()
        || metadata.group_note.is_some()
        || metadata.concurrency_limit.is_some()
        || metadata.enabled.is_some()
        || metadata.is_mother.is_some()
        || metadata.tag_ids.is_some()
}

fn external_oauth_request_to_import_item(
    source_account_id: &str,
    oauth: &ExternalOauthCredentialsRequest,
) -> Result<ImportOauthCredentialFileRequest, (StatusCode, String)> {
    let id_token = normalize_required_secret(&oauth.id_token, "oauth.idToken")?;
    let claims = parse_chatgpt_jwt_claims(&id_token).map_err(internal_error_tuple)?;
    let account_id = claims
        .chatgpt_account_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "oauth.idToken must contain chatgpt account id".to_string(),
            )
        })?;

    let content = json!({
        "type": "codex",
        "email": oauth.email,
        "account_id": account_id,
        "access_token": oauth.access_token,
        "refresh_token": oauth.refresh_token,
        "id_token": id_token,
        "token_type": oauth.token_type,
        "expired": oauth.expired.clone().unwrap_or_default(),
    });
    Ok(ImportOauthCredentialFileRequest {
        source_id: source_account_id.to_string(),
        file_name: format!("{source_account_id}.json"),
        content: content.to_string(),
    })
}

fn normalize_external_oauth_credentials(
    source_account_id: &str,
    oauth: &ExternalOauthCredentialsRequest,
) -> Result<NormalizedImportedOauthCredentials, (StatusCode, String)> {
    let import_item = external_oauth_request_to_import_item(source_account_id, oauth)?;
    normalize_imported_oauth_credentials(&import_item)
        .map_err(|message| (StatusCode::BAD_REQUEST, message))
}

async fn probe_external_oauth_credentials(
    state: &AppState,
    identity: &ExternalAccountIdentity,
    metadata: &ExternalUpstreamAccountMetadataRequest,
    existing_row: Option<&UpstreamAccountRow>,
    normalized: &NormalizedImportedOauthCredentials,
) -> Result<ImportedOauthProbeOutcome, (StatusCode, String)> {
    let binding = resolve_external_group_binding(state, metadata, existing_row).await?;
    let refresh_scope = binding
        .as_ref()
        .map(|value| {
            required_account_forward_proxy_scope(
                Some(&value.group_name),
                value.bound_proxy_keys.clone(),
            )
        })
        .transpose()
        .map_err(internal_error_tuple)?
        .unwrap_or(ForwardProxyRouteScope::Automatic);

    let usage_scope = match binding.as_ref() {
        Some(value) if value.node_shunt_enabled => {
            let assignments = build_upstream_account_node_shunt_assignments(state)
                .await
                .map_err(internal_error_tuple)?;
            resolve_group_forward_proxy_scope_for_provisioning(
                state,
                value,
                Some(&assignments),
                existing_row,
                &HashSet::new(),
            )
            .await
            .map_err(internal_error_tuple)?
        }
        Some(value) => required_account_forward_proxy_scope(
            Some(&value.group_name),
            value.bound_proxy_keys.clone(),
        )
        .map_err(internal_error_tuple)?,
        None => ForwardProxyRouteScope::Automatic,
    };
    let reservation_key = reserve_imported_oauth_node_shunt_scope(
        state,
        &identity.source_account_id,
        existing_row.map(|row| row.id),
        &usage_scope,
    )
    .map_err(internal_error_tuple)?;
    let probe = probe_imported_oauth_credentials(state, normalized, &refresh_scope, &usage_scope)
        .await
        .map_err(request_runtime_error_tuple);
    release_imported_oauth_node_shunt_scope(state, reservation_key);
    probe
}

pub(crate) async fn persist_external_existing_oauth_upsert(
    state: &AppState,
    identity: &ExternalAccountIdentity,
    account_id: i64,
    metadata: &ExternalUpstreamAccountMetadataRequest,
    probe: ImportedOauthProbeOutcome,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let existing_row = load_external_oauth_account_by_id(state, account_id, identity).await?;
    let plan =
        prepare_external_existing_oauth_upsert_plan(state, &existing_row, metadata, &probe).await?;
    let routing_scope_is_global = plan.routing_scope_is_global;
    persist_external_existing_oauth_account(state, identity, &existing_row, &probe, plan).await?;
    complete_external_existing_oauth_upsert(state, existing_row.id, &probe, routing_scope_is_global)
        .await
}

async fn prepare_external_existing_oauth_upsert_plan(
    state: &AppState,
    existing_row: &UpstreamAccountRow,
    metadata: &ExternalUpstreamAccountMetadataRequest,
    probe: &ImportedOauthProbeOutcome,
) -> Result<ExternalExistingOauthUpsertPlan, (StatusCode, String)> {
    let requested_display_name = metadata
        .display_name
        .as_deref()
        .map(normalize_required_display_name)
        .transpose()?;
    let target_group_name = normalize_external_group_name(metadata, Some(existing_row));
    let group_concurrency_limit =
        normalize_concurrency_limit(metadata.concurrency_limit, "concurrencyLimit")?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        RequestedGroupMetadataInput::from_external_metadata(metadata, group_concurrency_limit),
    );
    validate_group_note_target(target_group_name.as_deref(), metadata.group_note.is_some())?;
    let resolved_group_binding =
        resolve_external_group_binding(state, metadata, Some(existing_row)).await?;
    let group_name = resolved_group_binding
        .as_ref()
        .map(|value| value.group_name.clone())
        .or(target_group_name);
    let routing_scope_is_global =
        existing_row.group_name != group_name || requested_group_metadata_changes.was_requested();
    let note = if metadata.note.is_some() {
        normalize_optional_text(metadata.note.clone())
    } else {
        existing_row.note.clone()
    };
    // Empty tagIds is a compatibility no-op. External metadata writes are not
    // allowed to clear preserved system tags.
    let tag_ids = match metadata.tag_ids.clone() {
        Some(values) => {
            reject_manual_tag_ids(&values)?;
            load_external_account_tag_ids(&state.pool, existing_row.id).await?
        }
        None => load_external_account_tag_ids(&state.pool, existing_row.id).await?,
    };
    let is_mother = metadata.is_mother.unwrap_or(existing_row.is_mother != 0);
    let next_enabled = metadata.enabled.unwrap_or(existing_row.enabled != 0);
    let encrypted_credentials = encrypt_credentials(
        state.upstream_accounts.require_crypto_key()?,
        &StoredCredentials::Oauth(probe.credentials.clone()),
    )
    .map_err(internal_error_tuple)?;
    let next_verified_email = normalize_email_value(probe.claims.email.clone());
    let should_follow_verified_email = should_replace_email_after_verified_email_change(
        existing_row.email.as_deref(),
        existing_row.verified_email.as_deref(),
    );
    let chosen_email = if should_follow_verified_email {
        next_verified_email
            .clone()
            .or_else(|| existing_row.email.clone())
    } else {
        existing_row.email.clone()
    };
    let previous_display_email_source = if should_follow_verified_email {
        existing_row
            .email
            .as_deref()
            .or(existing_row.verified_email.as_deref())
    } else {
        existing_row.email.as_deref()
    };
    let display_name = resolve_display_name_after_email_change(
        Some(existing_row.display_name.as_str()),
        requested_display_name.as_deref(),
        previous_display_email_source,
        chosen_email.as_deref(),
    )
    .unwrap_or(existing_row.display_name.clone());
    Ok(ExternalExistingOauthUpsertPlan {
        display_name,
        chosen_email,
        verified_email: next_verified_email,
        group_name,
        is_mother,
        note,
        tag_ids,
        requested_group_metadata_changes,
        encrypted_credentials,
        next_enabled,
        routing_scope_is_global,
    })
}

async fn persist_external_existing_oauth_account(
    state: &AppState,
    identity: &ExternalAccountIdentity,
    existing_row: &UpstreamAccountRow,
    probe: &ImportedOauthProbeOutcome,
    plan: ExternalExistingOauthUpsertPlan,
) -> Result<(), (StatusCode, String)> {
    let ExternalExistingOauthUpsertPlan {
        display_name,
        chosen_email,
        verified_email,
        group_name,
        is_mother,
        note,
        tag_ids,
        requested_group_metadata_changes,
        encrypted_credentials,
        next_enabled,
        routing_scope_is_global: _,
    } = plan;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let effective_existing_plan_type = resolve_effective_plan_type_for_account(
        tx.as_mut(),
        existing_row.id,
        existing_row.plan_type.as_deref(),
    )
    .await
    .map_err(internal_error_tuple)?;
    ensure_display_name_available_for_oauth_identity(
        tx.as_mut(),
        &display_name,
        Some(existing_row.id),
        probe.claims.chatgpt_account_id.as_deref(),
        probe.claims.chatgpt_user_id.as_deref(),
        group_name.as_deref(),
        normalize_plan_type(probe.claims.chatgpt_plan_type.as_deref())
            .or(effective_existing_plan_type)
            .as_deref(),
    )
    .await?;
    upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(existing_row.id),
            display_name: &display_name,
            chosen_email,
            verified_email,
            group_name,
            is_mother,
            note,
            tag_ids,
            requested_group_metadata_changes,
            claims: &probe.claims,
            encrypted_credentials,
            has_refresh_token: oauth_credentials_have_refresh_token(&probe.credentials),
            token_expires_at: &probe.token_expires_at,
            external_identity: Some(identity),
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    if next_enabled != (existing_row.enabled != 0) {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET enabled = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(existing_row.id)
        .bind(if next_enabled { 1 } else { 0 })
        .bind(format_utc_iso(Utc::now()))
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(())
}

async fn complete_external_existing_oauth_upsert(
    state: &AppState,
    account_id: i64,
    probe: &ImportedOauthProbeOutcome,
    routing_scope_is_global: bool,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let _warning = apply_imported_oauth_probe_result(state, account_id, probe)
        .await
        .map_err(internal_error_tuple)?;
    let routing_scope = if routing_scope_is_global {
        None
    } else {
        Some(std::slice::from_ref(&account_id))
    };
    let routing_state_version =
        match publish_account_effective_routing_rules_changed(state, routing_scope, &[]).await {
            Ok(version) => Some(version),
            Err(err) => {
                warn!(
                    ?err,
                    account_id, "external OAuth update committed but routing publication failed"
                );
                invalidate_dashboard_activity_snapshots_with_accounts(
                    state.dashboard_activity_snapshot_cache.as_ref(),
                    "account_effective_routing_rules_publication_failed",
                )
                .await;
                None
            }
        };
    let mut detail = load_upstream_account_detail_with_actual_usage(state, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    detail.routing_state_version = routing_state_version;
    Ok(detail)
}

async fn load_external_oauth_account_by_id(
    state: &AppState,
    account_id: i64,
    identity: &ExternalAccountIdentity,
) -> Result<UpstreamAccountRow, (StatusCode, String)> {
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    if row.external_client_id.as_deref() != Some(identity.client_id.as_str())
        || row.external_source_account_id.as_deref() != Some(identity.source_account_id.as_str())
    {
        return Err((
            StatusCode::CONFLICT,
            "external source binding changed during update".to_string(),
        ));
    }
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        return Err((
            StatusCode::CONFLICT,
            "external source binding does not point to an OAuth account".to_string(),
        ));
    }
    Ok(row)
}

async fn load_external_oauth_account(
    state: &AppState,
    identity: &ExternalAccountIdentity,
) -> Result<UpstreamAccountRow, (StatusCode, String)> {
    let row = load_upstream_account_row_by_external_identity(
        &state.pool,
        &identity.client_id,
        &identity.source_account_id,
    )
    .await
    .map_err(internal_error_tuple)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "external upstream account not found".to_string(),
        )
    })?;
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        return Err((
            StatusCode::CONFLICT,
            "external source binding does not point to an OAuth account".to_string(),
        ));
    }
    Ok(row)
}

pub(crate) async fn external_upsert_oauth_upstream_account(
    state: Arc<AppState>,
    identity: ExternalAccountIdentity,
    payload: ExternalUpstreamAccountUpsertRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let prepared = prepare_external_oauth_upsert(state.as_ref(), identity, payload).await?;
    if let Some(account_id) = prepared.existing_account_id {
        return state
            .upstream_accounts
            .account_ops
            .run_external_oauth_upsert(
                state,
                account_id,
                prepared.identity,
                prepared.metadata,
                prepared.probe,
            )
            .await;
    }
    match persist_external_new_oauth_account(state.as_ref(), &prepared).await? {
        ExternalOauthPersistenceResult::Created(account_id) => {
            complete_external_new_oauth_upsert(state, account_id, prepared.probe).await
        }
        ExternalOauthPersistenceResult::Existing(detail) => Ok(detail),
    }
}

async fn prepare_external_oauth_upsert(
    state: &AppState,
    identity: ExternalAccountIdentity,
    payload: ExternalUpstreamAccountUpsertRequest,
) -> Result<PreparedExternalOauthUpsert, (StatusCode, String)> {
    let identity = ExternalAccountIdentity {
        client_id: identity.client_id,
        source_account_id: normalize_external_source_account_id(&identity.source_account_id)?,
    };
    let existing_row = load_upstream_account_row_by_external_identity(
        &state.pool,
        &identity.client_id,
        &identity.source_account_id,
    )
    .await
    .map_err(internal_error_tuple)?;
    if let Some(row) = existing_row.as_ref()
        && row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX
    {
        return Err((
            StatusCode::CONFLICT,
            "external source binding does not point to an OAuth account".to_string(),
        ));
    }
    let normalized =
        normalize_external_oauth_credentials(&identity.source_account_id, &payload.oauth)?;
    let probe = probe_external_oauth_credentials(
        state,
        &identity,
        &payload.metadata,
        existing_row.as_ref(),
        &normalized,
    )
    .await?;
    Ok(PreparedExternalOauthUpsert {
        identity,
        metadata: payload.metadata,
        normalized,
        probe,
        existing_account_id: existing_row.map(|row| row.id),
    })
}

async fn prepare_external_oauth_create_plan(
    state: &AppState,
    metadata: &ExternalUpstreamAccountMetadataRequest,
    probe: &ImportedOauthProbeOutcome,
) -> Result<ExternalOauthCreatePlan, (StatusCode, String)> {
    let target_group_name = normalize_external_group_name(metadata, None);
    let group_concurrency_limit =
        normalize_concurrency_limit(metadata.concurrency_limit, "concurrencyLimit")?;
    let requested_group_metadata_changes = build_requested_group_metadata_changes(
        RequestedGroupMetadataInput::from_external_metadata(metadata, group_concurrency_limit),
    );
    validate_group_note_target(target_group_name.as_deref(), metadata.group_note.is_some())?;
    let resolved_group_binding = resolve_external_group_binding(state, metadata, None).await?;
    let group_name = resolved_group_binding
        .as_ref()
        .map(|value| value.group_name.clone())
        .or(target_group_name);
    let note = normalize_optional_text(metadata.note.clone());
    // New external imports no longer accept manual tag mutation; keep the
    // stored contract stable by tolerating an explicit empty array.
    if let Some(values) = metadata.tag_ids.clone() {
        reject_manual_tag_ids(&values)?;
    }
    let encrypted_credentials = encrypt_credentials(
        state.upstream_accounts.require_crypto_key()?,
        &StoredCredentials::Oauth(probe.credentials.clone()),
    )
    .map_err(internal_error_tuple)?;
    Ok(ExternalOauthCreatePlan {
        group_name,
        is_mother: metadata.is_mother.unwrap_or(false),
        note,
        tag_ids: Vec::new(),
        requested_group_metadata_changes,
        encrypted_credentials,
        next_enabled: metadata.enabled.unwrap_or(true),
    })
}

async fn persist_external_new_oauth_account(
    state: &AppState,
    prepared: &PreparedExternalOauthUpsert,
) -> Result<ExternalOauthPersistenceResult, (StatusCode, String)> {
    let plan =
        prepare_external_oauth_create_plan(state, &prepared.metadata, &prepared.probe).await?;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    if let Some(existing_row) = load_upstream_account_row_by_external_identity_conn(
        tx.as_mut(),
        &prepared.identity.client_id,
        &prepared.identity.source_account_id,
    )
    .await
    .map_err(internal_error_tuple)?
    {
        drop(tx);
        let reprobe = probe_external_oauth_credentials(
            state,
            &prepared.identity,
            &prepared.metadata,
            Some(&existing_row),
            &prepared.normalized,
        )
        .await?;
        return persist_external_existing_oauth_upsert(
            state,
            &prepared.identity,
            existing_row.id,
            &prepared.metadata,
            reprobe,
        )
        .await
        .map(ExternalOauthPersistenceResult::Existing);
    }
    let display_name = resolve_external_create_display_name(
        &mut tx,
        &prepared.identity,
        &prepared.metadata,
        &prepared.normalized,
    )
    .await?;
    let ExternalOauthCreatePlan {
        group_name,
        is_mother,
        note,
        tag_ids,
        requested_group_metadata_changes,
        encrypted_credentials,
        next_enabled,
    } = plan;
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: &display_name,
            chosen_email: normalize_email_value(prepared.probe.claims.email.clone()),
            verified_email: normalize_email_value(prepared.probe.claims.email.clone()),
            group_name,
            is_mother,
            note,
            tag_ids,
            requested_group_metadata_changes,
            claims: &prepared.probe.claims,
            encrypted_credentials,
            has_refresh_token: oauth_credentials_have_refresh_token(&prepared.probe.credentials),
            token_expires_at: &prepared.probe.token_expires_at,
            external_identity: Some(&prepared.identity),
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    if !next_enabled {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET enabled = 0,
                updated_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(format_utc_iso(Utc::now()))
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(ExternalOauthPersistenceResult::Created(account_id))
}

async fn complete_external_new_oauth_upsert(
    state: Arc<AppState>,
    account_id: i64,
    probe: ImportedOauthProbeOutcome,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let _warning = state
        .upstream_accounts
        .account_ops
        .run_persist_imported_oauth(state.clone(), account_id, probe)
        .await?;
    // The imported OAuth command publishes the post-commit routing snapshot. Reuse its
    // generation for the detail response without advancing the cache a second time.
    let routing_state_version = current_routing_state_version(state.as_ref());
    publish_new_account_routing_availability_if_selectable(state.as_ref(), account_id).await;

    let mut detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    detail.routing_state_version = routing_state_version;
    Ok(detail)
}

pub(crate) async fn external_patch_oauth_upstream_account(
    state: Arc<AppState>,
    identity: ExternalAccountIdentity,
    payload: ExternalUpstreamAccountMetadataRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let identity = ExternalAccountIdentity {
        client_id: identity.client_id,
        source_account_id: normalize_external_source_account_id(&identity.source_account_id)?,
    };
    let row = load_external_oauth_account(state.as_ref(), &identity).await?;
    if !external_metadata_has_any_change(&payload) {
        return load_upstream_account_detail_with_actual_usage(state.as_ref(), row.id)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            row.id,
            external_metadata_to_update_request(payload),
        )
        .await
}

pub(crate) async fn external_relogin_oauth_upstream_account(
    state: Arc<AppState>,
    identity: ExternalAccountIdentity,
    payload: ExternalUpstreamAccountReloginRequest,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let identity = ExternalAccountIdentity {
        client_id: identity.client_id,
        source_account_id: normalize_external_source_account_id(&identity.source_account_id)?,
    };
    let row = load_external_oauth_account(state.as_ref(), &identity).await?;
    let normalized =
        normalize_external_oauth_credentials(&identity.source_account_id, &payload.oauth)?;
    let probe = probe_external_oauth_credentials(
        state.as_ref(),
        &identity,
        &ExternalUpstreamAccountMetadataRequest::default(),
        Some(&row),
        &normalized,
    )
    .await?;
    let _warning = state
        .upstream_accounts
        .account_ops
        .run_persist_imported_oauth(state.clone(), row.id, probe)
        .await?;
    state
        .upstream_accounts
        .account_ops
        .run_post_create_sync(state.clone(), row.id)
        .await
        .map_err(request_runtime_error_tuple)
}
