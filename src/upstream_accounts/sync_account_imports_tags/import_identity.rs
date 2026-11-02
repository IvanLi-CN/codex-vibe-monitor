use super::*;
use sqlx::Transaction;

pub(crate) async fn persist_imported_oauth_existing_inner_with_versions(
    state: &AppState,
    account_id: i64,
    probe: ImportedOauthProbeOutcome,
    expected_updated_at: Option<&str>,
    expected_usage_version: Option<Option<UsageSnapshotVersion>>,
) -> Result<Option<String>, (StatusCode, String)> {
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(account_id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    let existing_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    if expected_updated_at.is_some_and(|expected| existing_row.updated_at != expected) {
        return Err((
            StatusCode::CONFLICT,
            "account changed while OAuth credentials were being probed; retry the operation"
                .to_string(),
        ));
    }
    let existing_tag_ids = load_account_tag_map(&state.pool, &[account_id])
        .await
        .map_err(internal_error_tuple)?
        .remove(&account_id)
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.id)
        .collect::<Vec<_>>();
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(probe.credentials.clone()),
    )
    .map_err(internal_error_tuple)?;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    if let Some(expected_updated_at) = expected_updated_at {
        let current_updated_at = sqlx::query_scalar::<_, String>(
            "SELECT updated_at FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(existing_row.id)
        .fetch_optional(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
        if current_updated_at.as_deref() != Some(expected_updated_at) {
            return Err((
                StatusCode::CONFLICT,
                "account changed while OAuth credentials were being probed; retry the operation"
                    .to_string(),
            ));
        }
    }
    ensure_usage(
        tx.as_mut(),
        existing_row.id,
        expected_usage_version.as_ref(),
    )
    .await?;
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
    .bind(existing_row.id)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS)
    .bind(format_utc_iso(Utc::now()))
    .execute(tx.as_mut())
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
        &mut *tx,
        &existing_row.display_name,
        Some(existing_row.id),
        probe.claims.chatgpt_account_id.as_deref(),
        probe.claims.chatgpt_user_id.as_deref(),
        existing_row.group_name.as_deref(),
        normalize_plan_type(probe.claims.chatgpt_plan_type.as_deref())
            .or(effective_existing_plan_type)
            .as_deref(),
    )
    .await?;
    upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(existing_row.id),
            display_name: &existing_row.display_name,
            chosen_email: existing_row.email.clone(),
            verified_email: normalize_email_value(probe.claims.email.clone()),
            group_name: existing_row.group_name.clone(),
            is_mother: existing_row.is_mother != 0,
            note: existing_row.note.clone(),
            tag_ids: existing_tag_ids,
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &probe.claims,
            encrypted_credentials,
            has_refresh_token: oauth_credentials_have_refresh_token(&probe.credentials),
            token_expires_at: &probe.token_expires_at,
            external_identity: None,
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    let lifecycle_observed_at = format_utc_iso(Utc::now());
    let refresh_token_expiry_state = match probe.refresh_token_expiry_source.as_str() {
        "imported" => "known",
        "unparseable" => "unparseable",
        _ => "not_provided",
    };
    let access_token_expiry_source =
        if parse_access_token_expiry(&probe.credentials.access_token).is_some() {
            "jwt_exp"
        } else {
            "imported"
        };
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET access_token_expiry_source = ?2,
            access_token_expiry_observed_at = ?3,
            refresh_token_expires_at = CASE WHEN ?7 = 1 THEN ?4 ELSE refresh_token_expires_at END,
            refresh_token_expiry_source = CASE WHEN ?7 = 1 THEN ?5 ELSE refresh_token_expiry_source END,
            refresh_token_expiry_state = CASE WHEN ?7 = 1 THEN ?6 ELSE refresh_token_expiry_state END,
            refresh_token_expiry_observed_at = CASE WHEN ?7 = 1 THEN ?3 ELSE refresh_token_expiry_observed_at END,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(existing_row.id)
    .bind(access_token_expiry_source)
    .bind(&lifecycle_observed_at)
    .bind(probe.refresh_token_expires_at.clone())
    .bind(&probe.refresh_token_expiry_source)
    .bind(refresh_token_expiry_state)
    .bind(i64::from(probe.refresh_token_metadata_update))
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?;
    if existing_row.enabled != 1 {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET enabled = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(existing_row.id)
        .bind(existing_row.enabled)
        .bind(format_utc_iso(Utc::now()))
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;

    let next_attempt_at = parse_rfc3339_utc(&probe.token_expires_at).map(|expires| {
        format_utc_iso(
            expires
                - ChronoDuration::seconds(
                    state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                ),
        )
    });
    complete_upstream_account_refresh_success(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT,
        next_attempt_at.as_deref(),
    )
    .await
    .map_err(internal_error_tuple)?;

    let post_import_warning = match apply_imported_oauth_probe_result_locked_with_expected_usage(
        state,
        account_id,
        &probe,
        expected_usage_version,
    )
    .await
    {
        Ok(warning) => warning,
        Err(err)
            if err
                .chain()
                .any(|cause| cause.to_string() == OAUTH_USAGE_SNAPSHOT_CONFLICT_MESSAGE) =>
        {
            return Err((
                StatusCode::CONFLICT,
                "usage data changed while OAuth credentials were being probed; retry the operation"
                    .to_string(),
            ));
        }
        Err(err) => {
            warn!(
                account_id,
                error = %err,
                "imported OAuth credential persisted but post-import state update failed"
            );
            Some(format!(
                "Imported, but post-import state update failed: {err}"
            ))
        }
    };
    if let Err(err) = publish_account_effective_routing_rules_changed(state, None, &[]).await {
        warn!(
            ?err,
            account_id, "imported OAuth write committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
    Ok(post_import_warning)
}

pub(crate) struct OauthAccountUpsert<'a> {
    pub(crate) account_id: Option<i64>,
    pub(crate) display_name: &'a str,
    pub(crate) chosen_email: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) group_name: Option<String>,
    pub(crate) is_mother: bool,
    pub(crate) note: Option<String>,
    pub(crate) tag_ids: Vec<i64>,
    pub(crate) requested_group_metadata_changes: RequestedGroupMetadataChanges,
    pub(crate) claims: &'a ChatgptJwtClaims,
    pub(crate) encrypted_credentials: String,
    pub(crate) has_refresh_token: bool,
    pub(crate) token_expires_at: &'a str,
    pub(crate) external_identity: Option<&'a ExternalAccountIdentity>,
}

pub(crate) fn duplicate_display_name_error() -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        "displayName must be unique".to_string(),
    )
}

pub(crate) async fn load_conflicting_display_name_id(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE lower(trim(display_name)) = lower(trim(?1))
          AND deleted_at IS NULL
          AND (?2 IS NULL OR id != ?2)
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(display_name)
    .bind(exclude_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct DisplayNameConflictRow {
    id: i64,
    kind: String,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
}

pub(crate) fn normalize_display_name_or_email_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

pub(crate) fn normalize_email_value(value: Option<String>) -> Option<String> {
    let normalized = normalize_optional_text(value)?;
    BASIC_EMAIL_REGEX
        .is_match(&normalized)
        .then(|| normalized.to_ascii_lowercase())
}

pub(crate) fn normalize_optional_email(
    value: Option<String>,
    field_name: &str,
) -> Result<Option<String>, (StatusCode, String)> {
    let normalized = normalize_optional_text(value);
    let Some(normalized) = normalized else {
        return Ok(None);
    };
    if !BASIC_EMAIL_REGEX.is_match(&normalized) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be a valid email"),
        ));
    }
    Ok(Some(normalized.to_ascii_lowercase()))
}

pub(crate) fn generated_display_name_from_email(email: Option<&str>) -> Option<String> {
    normalize_display_name_or_email_key(email)
}

pub(crate) fn should_replace_display_name_after_email_change(
    display_name: Option<&str>,
    previous_email: Option<&str>,
) -> bool {
    let normalized_display_name = normalize_display_name_or_email_key(display_name);
    let Some(normalized_display_name) = normalized_display_name else {
        return true;
    };
    let Some(previous_generated_name) = generated_display_name_from_email(previous_email) else {
        return false;
    };
    normalized_display_name == previous_generated_name
}

pub(crate) fn resolve_display_name_after_email_change(
    previous_display_name: Option<&str>,
    requested_display_name: Option<&str>,
    previous_email: Option<&str>,
    next_email: Option<&str>,
) -> Option<String> {
    let normalized_previous = normalize_optional_text(previous_display_name.map(str::to_string));
    let normalized_requested = normalize_optional_text(requested_display_name.map(str::to_string));
    let requested_overrides_previous =
        normalize_display_name_or_email_key(normalized_requested.as_deref())
            != normalize_display_name_or_email_key(normalized_previous.as_deref())
            && normalized_requested.is_some();
    if requested_overrides_previous {
        return normalized_requested;
    }
    if should_replace_display_name_after_email_change(
        normalized_previous.as_deref(),
        previous_email,
    ) && let Some(generated) = generated_display_name_from_email(next_email)
    {
        return Some(generated);
    }
    normalized_requested.or(normalized_previous)
}

pub(crate) fn should_replace_email_after_verified_email_change(
    current_email: Option<&str>,
    previous_verified_email: Option<&str>,
) -> bool {
    let normalized_current = normalize_display_name_or_email_key(current_email);
    let normalized_previous_verified = normalize_display_name_or_email_key(previous_verified_email);
    normalized_current.is_none()
        || (normalized_current.is_some() && normalized_current == normalized_previous_verified)
}

pub(crate) async fn load_conflicting_display_name_rows(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
) -> Result<Vec<DisplayNameConflictRow>> {
    sqlx::query_as::<_, DisplayNameConflictRow>(
        r#"
        SELECT
            account.id,
            account.kind,
            account.chatgpt_account_id,
            account.chatgpt_user_id,
            account.group_name,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = account.id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = account.id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type
        FROM pool_upstream_accounts account
        WHERE lower(trim(account.display_name)) = lower(trim(?1))
          AND account.deleted_at IS NULL
          AND (?2 IS NULL OR account.id != ?2)
        ORDER BY account.id ASC
        "#,
    )
    .bind(display_name)
    .bind(exclude_id)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}

pub(crate) fn same_real_upstream_identity_for_display_name(
    current_chatgpt_account_id: Option<&str>,
    current_chatgpt_user_id: Option<&str>,
    current_group_name: Option<&str>,
    current_plan_type: Option<&str>,
    peer: &DisplayNameConflictRow,
) -> bool {
    let current_user_key = normalize_display_name_or_email_key(current_chatgpt_user_id);
    let peer_user_key = normalize_display_name_or_email_key(peer.chatgpt_user_id.as_deref());
    if current_user_key.is_some() && current_user_key == peer_user_key {
        return true;
    }

    let current_account_key = normalize_display_name_or_email_key(current_chatgpt_account_id);
    let peer_account_key = normalize_display_name_or_email_key(peer.chatgpt_account_id.as_deref());
    if current_account_key.is_none() || current_account_key != peer_account_key {
        return false;
    }

    let current_member = UpstreamAccountIdentityClusterMember {
        id: -1,
        chatgpt_user_id: normalize_optional_text(current_chatgpt_user_id.map(str::to_string)),
        group_name: normalize_legacy_ungrouped_group_name(current_group_name.map(str::to_string)),
        plan_type: normalize_plan_type(current_plan_type),
    };
    let peer_member = UpstreamAccountIdentityClusterMember {
        id: peer.id,
        chatgpt_user_id: normalize_optional_text(peer.chatgpt_user_id.clone()),
        group_name: normalize_legacy_ungrouped_group_name(peer.group_name.clone()),
        plan_type: normalize_plan_type(peer.plan_type.as_deref()),
    };
    !is_team_shared_org_peer_pair(&current_member, &peer_member)
}

pub(crate) fn is_mixed_plan_same_upstream_display_name_exempt(
    current_chatgpt_account_id: Option<&str>,
    current_chatgpt_user_id: Option<&str>,
    current_group_name: Option<&str>,
    current_plan_type: Option<&str>,
    peer: &DisplayNameConflictRow,
) -> bool {
    if peer.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        return false;
    }
    if !same_real_upstream_identity_for_display_name(
        current_chatgpt_account_id,
        current_chatgpt_user_id,
        current_group_name,
        current_plan_type,
        peer,
    ) {
        return false;
    }
    match (
        normalize_plan_type(current_plan_type),
        normalize_plan_type(peer.plan_type.as_deref()),
    ) {
        (Some(current_plan), Some(peer_plan)) => !current_plan.eq_ignore_ascii_case(&peer_plan),
        _ => false,
    }
}

pub(crate) async fn ensure_display_name_available(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
) -> Result<(), (StatusCode, String)> {
    let conflict = load_conflicting_display_name_id(executor, display_name, exclude_id)
        .await
        .map_err(internal_error_tuple)?;
    if conflict.is_some() {
        return Err(duplicate_display_name_error());
    }
    Ok(())
}

pub(crate) async fn ensure_display_name_available_for_oauth_identity(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
    chatgpt_account_id: Option<&str>,
    chatgpt_user_id: Option<&str>,
    group_name: Option<&str>,
    plan_type: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    let conflicts = load_conflicting_display_name_rows(executor, display_name, exclude_id)
        .await
        .map_err(internal_error_tuple)?;
    if conflicts.into_iter().any(|conflict| {
        !is_mixed_plan_same_upstream_display_name_exempt(
            chatgpt_account_id,
            chatgpt_user_id,
            group_name,
            plan_type,
            &conflict,
        )
    }) {
        return Err(duplicate_display_name_error());
    }
    Ok(())
}

pub(crate) async fn upsert_oauth_account(
    tx: &mut Transaction<'_, Sqlite>,
    payload: OauthAccountUpsert<'_>,
) -> Result<i64> {
    let OauthAccountUpsert {
        account_id,
        display_name,
        chosen_email,
        verified_email,
        group_name,
        is_mother,
        note,
        tag_ids,
        requested_group_metadata_changes,
        claims,
        encrypted_credentials,
        has_refresh_token,
        token_expires_at,
        external_identity,
    } = payload;
    let group_name = Some(normalize_upstream_account_group_name(group_name));
    let target_group_name = group_name.clone();
    let now_iso = format_utc_iso(Utc::now());
    let resolved_account_id = account_id;
    let external_client_id = external_identity.map(|value| value.client_id.as_str());
    let external_source_account_id =
        external_identity.map(|value| value.source_account_id.as_str());

    if let Some(existing_id) = resolved_account_id {
        let previous_group_name = load_upstream_account_row_conn(tx.as_mut(), existing_id)
            .await?
            .and_then(|row| row.group_name);
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET kind = ?2,
                provider = ?3,
                display_name = ?4,
                group_name = COALESCE(?5, group_name),
                is_mother = ?6,
                note = ?7,
                status = ?8,
                enabled = 1,
                email = ?9,
                verified_email = ?10,
                chatgpt_account_id = ?11,
                chatgpt_user_id = ?12,
                plan_type = ?13,
                plan_type_observed_at = CASE
                    WHEN NULLIF(TRIM(?13), '') IS NOT NULL THEN ?16
                    ELSE plan_type_observed_at
                END,
                encrypted_credentials = ?14,
                token_expires_at = ?15,
                has_refresh_token = ?19,
                last_refreshed_at = ?16,
                external_client_id = COALESCE(?17, external_client_id),
                external_source_account_id = COALESCE(?18, external_source_account_id),
                last_error = NULL,
                last_error_at = NULL,
                updated_at = ?16
            WHERE id = ?1
            "#,
        )
        .bind(existing_id)
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(&group_name)
        .bind(if is_mother { 1 } else { 0 })
        .bind(note)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(chosen_email)
        .bind(verified_email)
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .bind(external_client_id)
        .bind(external_source_account_id)
        .bind(if has_refresh_token { 1 } else { 0 })
        .execute(tx.as_mut())
        .await?;
        save_group_metadata_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            &requested_group_metadata_changes,
            previous_group_name == target_group_name,
        )
        .await?;
        if previous_group_name != target_group_name {
            cleanup_orphaned_group_metadata(tx.as_mut(), previous_group_name.as_deref()).await?;
        }
        apply_mother_assignment(tx, existing_id, group_name.as_deref(), is_mother).await?;
        sync_account_tag_links_with_executor(tx.as_mut(), existing_id, &tag_ids).await?;
        Ok(existing_id)
    } else {
        let inserted_account_id: i64 = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, is_mother, note, status, enabled,
                email, verified_email, chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at,
                masked_api_key, encrypted_credentials, token_expires_at,
                has_refresh_token,
                last_refreshed_at, last_synced_at, last_successful_sync_at,
                last_error, last_error_at, local_primary_limit, local_secondary_limit,
                local_limit_unit, external_client_id, external_source_account_id,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1,
                ?8, ?9, ?10, ?11, ?12, ?13,
                NULL, ?14, ?15,
                ?19,
                ?16, NULL, NULL,
                NULL, NULL, NULL, NULL,
                NULL, ?17, ?18, ?16, ?16
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(&group_name)
        .bind(if is_mother { 1 } else { 0 })
        .bind(note)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(chosen_email)
        .bind(verified_email)
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(
            claims
                .chatgpt_plan_type
                .as_deref()
                .and_then(|value| (!value.trim().is_empty()).then_some(now_iso.clone())),
        )
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .bind(external_client_id)
        .bind(external_source_account_id)
        .bind(if has_refresh_token { 1 } else { 0 })
        .fetch_one(tx.as_mut())
        .await?;
        save_group_metadata_after_account_write(
            tx.as_mut(),
            target_group_name.as_deref(),
            &requested_group_metadata_changes,
            false,
        )
        .await?;
        apply_mother_assignment(tx, inserted_account_id, group_name.as_deref(), is_mother).await?;
        sync_account_tag_links_with_executor(tx.as_mut(), inserted_account_id, &tag_ids).await?;
        Ok(inserted_account_id)
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct UpstreamAccountIdentityRow {
    id: i64,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct UpstreamAccountIdentityClusterMember {
    id: i64,
    chatgpt_user_id: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
}

pub(crate) fn normalize_plan_type(plan_type: Option<&str>) -> Option<String> {
    plan_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn is_team_plan_type(plan_type: Option<&str>) -> bool {
    normalize_plan_type(plan_type)
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("team"))
}

pub(crate) fn should_flag_duplicate_identity_pair(
    _current_plan_type: Option<&str>,
    _peer_plan_type: Option<&str>,
) -> bool {
    true
}

pub(crate) fn is_team_shared_org_cluster_member(
    member: &UpstreamAccountIdentityClusterMember,
) -> bool {
    is_team_plan_type(member.plan_type.as_deref())
        && member.group_name.is_some()
        && member.chatgpt_user_id.is_some()
}

pub(crate) fn is_team_shared_org_peer_pair(
    current: &UpstreamAccountIdentityClusterMember,
    peer: &UpstreamAccountIdentityClusterMember,
) -> bool {
    is_team_shared_org_cluster_member(current)
        && is_team_shared_org_cluster_member(peer)
        && current.group_name == peer.group_name
        && current.chatgpt_user_id != peer.chatgpt_user_id
}

pub(crate) fn should_flag_shared_account_id_duplicate_pair(
    current: &UpstreamAccountIdentityClusterMember,
    peer: &UpstreamAccountIdentityClusterMember,
) -> bool {
    if is_team_shared_org_peer_pair(current, peer) {
        return false;
    }
    !matches!(
        (
        current.chatgpt_user_id.as_deref(),
        peer.chatgpt_user_id.as_deref(),
    ),
        (Some(current_user_id), Some(peer_user_id))
            if !current_user_id.eq_ignore_ascii_case(peer_user_id)
    )
}

pub(crate) fn resolve_effective_plan_type(
    account_plan_type: Option<&str>,
    sample_plan_type: Option<&str>,
) -> Option<String> {
    normalize_plan_type(sample_plan_type).or_else(|| normalize_plan_type(account_plan_type))
}

pub(crate) async fn load_latest_usage_sample_plan_type_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    account_id: i64,
) -> Result<Option<String>> {
    let sample_plan_type = sqlx::query_scalar::<_, String>(
        r#"
        SELECT TRIM(plan_type)
        FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
          AND plan_type IS NOT NULL
          AND TRIM(plan_type) <> ''
        ORDER BY captured_at DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_optional(executor)
    .await?;
    Ok(normalize_plan_type(sample_plan_type.as_deref()))
}

pub(crate) async fn resolve_effective_plan_type_for_account(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    account_id: i64,
    account_plan_type: Option<&str>,
) -> Result<Option<String>> {
    let latest_sample_plan_type =
        load_latest_usage_sample_plan_type_with_executor(executor, account_id).await?;
    Ok(resolve_effective_plan_type(
        account_plan_type,
        latest_sample_plan_type.as_deref(),
    ))
}

pub(crate) async fn resolve_snapshot_plan_type(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    snapshot: &NormalizedUsageSnapshot,
) -> Result<Option<String>> {
    if let Some(plan_type) = normalize_plan_type(snapshot.plan_type.as_deref()) {
        return Ok(Some(plan_type));
    }

    let latest_sample_plan_type = load_latest_usage_sample(pool, row.id)
        .await?
        .and_then(|sample| sample.plan_type);
    Ok(latest_sample_plan_type.or_else(|| normalize_plan_type(row.plan_type.as_deref())))
}

pub(crate) async fn load_duplicate_info_map(
    pool: &Pool<Sqlite>,
) -> Result<std::collections::HashMap<i64, DuplicateInfo>> {
    let rows = sqlx::query_as::<_, UpstreamAccountIdentityRow>(
        r#"
        SELECT
            account.id,
            account.chatgpt_account_id,
            account.chatgpt_user_id,
            account.group_name,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = account.id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = account.id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type
        FROM pool_upstream_accounts account
        WHERE account.kind = ?1
        ORDER BY id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .fetch_all(pool)
    .await?;

    let mut by_account_id =
        std::collections::HashMap::<String, Vec<UpstreamAccountIdentityClusterMember>>::new();
    let mut by_user_id =
        std::collections::HashMap::<String, Vec<UpstreamAccountIdentityClusterMember>>::new();
    for row in &rows {
        if let Some(chatgpt_account_id) = row.chatgpt_account_id.as_ref().cloned() {
            by_account_id.entry(chatgpt_account_id).or_default().push(
                UpstreamAccountIdentityClusterMember {
                    id: row.id,
                    chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
                    group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
                    plan_type: normalize_plan_type(row.plan_type.as_deref()),
                },
            );
        }
        if let Some(chatgpt_user_id) = row.chatgpt_user_id.as_ref().cloned() {
            by_user_id.entry(chatgpt_user_id).or_default().push(
                UpstreamAccountIdentityClusterMember {
                    id: row.id,
                    chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
                    group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
                    plan_type: normalize_plan_type(row.plan_type.as_deref()),
                },
            );
        }
    }

    let mut duplicate_info = std::collections::HashMap::new();
    for row in rows {
        let current_member = UpstreamAccountIdentityClusterMember {
            id: row.id,
            chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
            group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
            plan_type: normalize_plan_type(row.plan_type.as_deref()),
        };
        let mut peer_ids = std::collections::BTreeSet::new();
        let mut reasons = Vec::new();

        if let Some(chatgpt_account_id) = row.chatgpt_account_id.as_ref()
            && let Some(cluster) = by_account_id
                .get(chatgpt_account_id)
                .filter(|members| members.len() > 1)
        {
            for member in cluster {
                if member.id != row.id
                    && should_flag_shared_account_id_duplicate_pair(&current_member, member)
                {
                    peer_ids.insert(member.id);
                }
            }
            if cluster.iter().any(|member| {
                member.id != row.id
                    && should_flag_shared_account_id_duplicate_pair(&current_member, member)
            }) {
                reasons.push(DuplicateReason::SharedChatgptAccountId);
            }
        }

        if let Some(chatgpt_user_id) = row.chatgpt_user_id.as_ref()
            && let Some(cluster) = by_user_id
                .get(chatgpt_user_id)
                .filter(|members| members.len() > 1)
        {
            for member in cluster {
                if member.id != row.id
                    && should_flag_duplicate_identity_pair(
                        current_member.plan_type.as_deref(),
                        member.plan_type.as_deref(),
                    )
                {
                    peer_ids.insert(member.id);
                }
            }
            if cluster.iter().any(|member| {
                member.id != row.id
                    && should_flag_duplicate_identity_pair(
                        current_member.plan_type.as_deref(),
                        member.plan_type.as_deref(),
                    )
            }) {
                reasons.push(DuplicateReason::SharedChatgptUserId);
            }
        }

        if !peer_ids.is_empty() {
            duplicate_info.insert(
                row.id,
                DuplicateInfo {
                    peer_account_ids: peer_ids.into_iter().collect(),
                    reasons,
                },
            );
        }
    }

    Ok(duplicate_info)
}

pub(crate) async fn load_duplicate_info_for_account(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<DuplicateInfo>> {
    let current_row = sqlx::query_as::<_, UpstreamAccountIdentityRow>(
        r#"
        SELECT
            account.id,
            account.chatgpt_account_id,
            account.chatgpt_user_id,
            account.group_name,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = account.id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = account.id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type
        FROM pool_upstream_accounts account
        WHERE account.kind = ?1
          AND account.id = ?2
        LIMIT 1
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    let Some(current_row) = current_row else {
        return Ok(None);
    };

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account.id,
            account.chatgpt_account_id,
            account.chatgpt_user_id,
            account.group_name,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = account.id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = account.id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC, previous_sample.id DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type
        FROM pool_upstream_accounts account
        WHERE account.kind = 
        "#,
    );
    query
        .push_bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .push(" AND account.id <> ")
        .push_bind(account_id)
        .push(" AND (");
    let mut has_identity_filter = false;
    if let Some(chatgpt_account_id) = current_row.chatgpt_account_id.as_ref() {
        query
            .push("account.chatgpt_account_id = ")
            .push_bind(chatgpt_account_id);
        has_identity_filter = true;
    }
    if let Some(chatgpt_user_id) = current_row.chatgpt_user_id.as_ref() {
        if has_identity_filter {
            query.push(" OR ");
        }
        query
            .push("account.chatgpt_user_id = ")
            .push_bind(chatgpt_user_id);
        has_identity_filter = true;
    }
    if !has_identity_filter {
        return Ok(None);
    }
    query.push(")");

    let peer_rows = query
        .build_query_as::<UpstreamAccountIdentityRow>()
        .fetch_all(pool)
        .await?;
    if peer_rows.is_empty() {
        return Ok(None);
    }

    let current_member = UpstreamAccountIdentityClusterMember {
        id: current_row.id,
        chatgpt_user_id: normalize_optional_text(current_row.chatgpt_user_id.clone()),
        group_name: normalize_legacy_ungrouped_group_name(current_row.group_name.clone()),
        plan_type: normalize_plan_type(current_row.plan_type.as_deref()),
    };
    let mut peer_ids = std::collections::BTreeSet::new();
    let mut reasons = Vec::new();

    if let Some(chatgpt_account_id) = current_row.chatgpt_account_id.as_ref() {
        let matching_peers = peer_rows
            .iter()
            .filter(|row| row.chatgpt_account_id.as_ref() == Some(chatgpt_account_id))
            .collect::<Vec<_>>();
        if matching_peers.iter().any(|row| {
            let peer = UpstreamAccountIdentityClusterMember {
                id: row.id,
                chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
                group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
                plan_type: normalize_plan_type(row.plan_type.as_deref()),
            };
            should_flag_shared_account_id_duplicate_pair(&current_member, &peer)
        }) {
            reasons.push(DuplicateReason::SharedChatgptAccountId);
        }
        for row in matching_peers {
            let peer = UpstreamAccountIdentityClusterMember {
                id: row.id,
                chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
                group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
                plan_type: normalize_plan_type(row.plan_type.as_deref()),
            };
            if should_flag_shared_account_id_duplicate_pair(&current_member, &peer) {
                peer_ids.insert(row.id);
            }
        }
    }

    if let Some(chatgpt_user_id) = current_row.chatgpt_user_id.as_ref() {
        let matching_peers = peer_rows
            .iter()
            .filter(|row| row.chatgpt_user_id.as_ref() == Some(chatgpt_user_id))
            .collect::<Vec<_>>();
        if matching_peers.iter().any(|row| {
            should_flag_duplicate_identity_pair(
                current_member.plan_type.as_deref(),
                normalize_plan_type(row.plan_type.as_deref()).as_deref(),
            )
        }) {
            reasons.push(DuplicateReason::SharedChatgptUserId);
        }
        for row in matching_peers {
            if should_flag_duplicate_identity_pair(
                current_member.plan_type.as_deref(),
                normalize_plan_type(row.plan_type.as_deref()).as_deref(),
            ) {
                peer_ids.insert(row.id);
            }
        }
    }

    if peer_ids.is_empty() {
        return Ok(None);
    }

    Ok(Some(DuplicateInfo {
        peer_account_ids: peer_ids.into_iter().collect(),
        reasons,
    }))
}
