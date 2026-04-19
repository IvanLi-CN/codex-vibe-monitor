async fn sync_upstream_account_by_id(
    state: &AppState,
    id: i64,
    cause: SyncCause,
) -> Result<Option<UpstreamAccountDetail>> {
    let row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;

    if row.enabled == 0 {
        if cause == SyncCause::Manual {
            bail!("disabled accounts cannot be synced");
        }
        let detail = load_upstream_account_detail_with_actual_usage(state, id)
            .await?
            .ok_or_else(|| anyhow!("account not found"))?;
        return Ok(Some(detail));
    }

    let group_metadata =
        match resolve_pool_account_group_proxy_routing_readiness(state, row.group_name.as_deref())
            .await?
        {
            PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => group_metadata,
            PoolAccountGroupProxyRoutingReadiness::Blocked(message) => bail!(message),
        };
    let sync_result = match row.kind.as_str() {
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX => {
            if group_metadata.node_shunt_enabled {
                resolve_account_forward_proxy_scope_for_sync(
                    state,
                    &row,
                    Some(group_metadata.clone()),
                )
                .await?;
            }
            sync_oauth_account(state, &row, cause).await
        }
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX => {
            if group_metadata.node_shunt_enabled {
                resolve_account_forward_proxy_scope(state, &row, Some(group_metadata)).await?;
            }
            sync_api_key_account(&state.pool, &row, cause).await
        }
        _ => bail!("unsupported account kind: {}", row.kind),
    };
    if let Err(err) = sync_result {
        return Err(err);
    }

    let detail = load_upstream_account_detail_with_actual_usage(state, id)
        .await?
        .ok_or_else(|| anyhow!("account not found after sync"))?;
    Ok(Some(detail))
}

async fn sync_api_key_account(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    cause: SyncCause,
) -> Result<()> {
    let sync_source = sync_cause_action_source(cause);
    if row.status != UPSTREAM_ACCOUNT_STATUS_ACTIVE
        && route_failure_kind_requires_manual_api_key_recovery(
            row.last_route_failure_kind.as_deref(),
        )
    {
        let reason_message = if route_failure_kind_is_quota_exhausted(
            row.last_route_failure_kind.as_deref(),
        ) {
            "manual recovery required because API key sync cannot verify whether the upstream usage limit has reset"
        } else {
            "manual recovery required because API key sync cannot verify whether upstream credentials or entitlements have recovered"
        };
        return record_account_sync_recovery_blocked(
            pool,
            row.id,
            sync_source,
            &row.status,
            UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED,
            reason_message,
            row.last_error.as_deref(),
            row.last_route_failure_kind.as_deref(),
        )
        .await;
    }
    mark_account_sync_success(
        pool,
        row.id,
        sync_source,
        if should_clear_route_failure_state_after_sync_success(row) {
            SyncSuccessRouteState::ClearFailureState
        } else {
            SyncSuccessRouteState::PreserveFailureState
        },
    )
    .await
}

async fn sync_oauth_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    cause: SyncCause,
) -> Result<()> {
    let sync_source = sync_cause_action_source(cause);
    let now = Utc::now();
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .ok_or_else(|| anyhow!("account writes are disabled"))?;
    let decrypted = decrypt_credentials(
        crypto_key,
        row.encrypted_credentials
            .as_deref()
            .ok_or_else(|| anyhow!("missing encrypted OAuth credentials"))?,
    )?;
    let StoredCredentials::Oauth(mut credentials) = decrypted else {
        bail!("unexpected credential kind for OAuth account")
    };

    let expires_at = row.token_expires_at.as_deref().and_then(parse_rfc3339_utc);
    let refresh_due = expires_at
        .map(|expires| {
            expires
                <= now
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);
    let usage_scope = match resolve_account_forward_proxy_scope_for_sync(state, row, None).await {
        Ok(scope) => scope,
        Err(err) if is_group_node_shunt_unassigned_message(&err.to_string()) => {
            return Err(err);
        }
        Err(err) => {
            record_classified_account_sync_failure(&state.pool, row, sync_source, &err.to_string())
                .await?;
            return Ok(());
        }
    };
    let refresh_scope = usage_scope.clone();
    set_account_status(&state.pool, row.id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None).await?;

    if refresh_due {
        match refresh_oauth_tokens_for_required_scope(
            state,
            &refresh_scope,
            &credentials.refresh_token,
        )
        .await
        {
            Ok(response) => {
                credentials.access_token = response.access_token;
                if let Some(refresh_token) = response.refresh_token {
                    credentials.refresh_token = refresh_token;
                }
                if let Some(id_token) = response.id_token {
                    credentials.id_token = id_token;
                }
                credentials.token_type = response.token_type;
                let token_expires_at = format_utc_iso(
                    Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                );
                persist_oauth_credentials(
                    &state.pool,
                    row.id,
                    crypto_key,
                    &credentials,
                    &token_expires_at,
                )
                .await?;
            }
            Err(err) if is_reauth_error(&err) => {
                record_account_sync_failure(
                    &state.pool,
                    row.id,
                    sync_source,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    &err.to_string(),
                    UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
                    None,
                    PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                    None,
                    false,
                )
                .await?;
                return Ok(());
            }
            Err(err) => {
                let (disposition, reason_code, next_status, http_status, failure_kind) =
                    classify_sync_failure(&row.kind, &err.to_string());
                let next_status = match disposition {
                    UpstreamAccountFailureDisposition::HardUnavailable => {
                        next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR)
                    }
                    UpstreamAccountFailureDisposition::RateLimited
                    | UpstreamAccountFailureDisposition::Retryable => {
                        UPSTREAM_ACCOUNT_STATUS_ACTIVE
                    }
                };
                let route_failure_kind = match disposition {
                    UpstreamAccountFailureDisposition::HardUnavailable => Some(failure_kind),
                    UpstreamAccountFailureDisposition::RateLimited
                    | UpstreamAccountFailureDisposition::Retryable => None,
                };
                record_account_sync_failure(
                    &state.pool,
                    row.id,
                    sync_source,
                    next_status,
                    &err.to_string(),
                    reason_code,
                    http_status,
                    failure_kind,
                    route_failure_kind,
                    disposition == UpstreamAccountFailureDisposition::HardUnavailable,
                )
                .await?;
                return Ok(());
            }
        }
    }

    let mut latest_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared during sync"))?;
    let decrypted = decrypt_credentials(
        crypto_key,
        latest_row
            .encrypted_credentials
            .as_deref()
            .ok_or_else(|| anyhow!("missing encrypted OAuth credentials"))?,
    )?;
    let StoredCredentials::Oauth(credentials) = decrypted else {
        bail!("unexpected credential kind for OAuth account")
    };

    let usage_result = fetch_usage_snapshot_via_forward_proxy(
        state,
        &usage_scope,
        &state.config,
        &credentials.access_token,
        latest_row.chatgpt_account_id.as_deref(),
    )
    .await;

    let snapshot = match usage_result {
        Ok(snapshot) => snapshot,
        Err(err) if err.to_string().contains("401") || err.to_string().contains("403") => {
            match refresh_oauth_tokens_for_required_scope(
                state,
                &refresh_scope,
                &credentials.refresh_token,
            )
            .await
            {
                Ok(response) => {
                    let mut refreshed = credentials.clone();
                    refreshed.access_token = response.access_token;
                    if let Some(refresh_token) = response.refresh_token {
                        refreshed.refresh_token = refresh_token;
                    }
                    if let Some(id_token) = response.id_token {
                        refreshed.id_token = id_token;
                    }
                    refreshed.token_type = response.token_type;
                    let token_expires_at = format_utc_iso(
                        Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                    );
                    persist_oauth_credentials(
                        &state.pool,
                        row.id,
                        crypto_key,
                        &refreshed,
                        &token_expires_at,
                    )
                    .await?;
                    latest_row = load_upstream_account_row(&state.pool, row.id)
                        .await?
                        .ok_or_else(|| anyhow!("account disappeared during retry refresh"))?;
                    match fetch_usage_snapshot_via_forward_proxy(
                        state,
                        &usage_scope,
                        &state.config,
                        &refreshed.access_token,
                        latest_row.chatgpt_account_id.as_deref(),
                    )
                    .await
                    {
                        Ok(snapshot) => snapshot,
                        Err(retry_err) => {
                            record_classified_account_sync_failure(
                                &state.pool,
                                &latest_row,
                                sync_source,
                                &retry_err.to_string(),
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                }
                Err(refresh_err) if is_reauth_error(&refresh_err) => {
                    record_account_sync_failure(
                        &state.pool,
                        row.id,
                        sync_source,
                        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                        &refresh_err.to_string(),
                        UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
                        None,
                        PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                        None,
                        false,
                    )
                    .await?;
                    return Ok(());
                }
                Err(refresh_err) => {
                    record_classified_account_sync_failure(
                        &state.pool,
                        &latest_row,
                        sync_source,
                        &refresh_err.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
        Err(err) => {
            record_classified_account_sync_failure(
                &state.pool,
                &latest_row,
                sync_source,
                &err.to_string(),
            )
            .await?;
            return Ok(());
        }
    };

    let effective_snapshot_plan_type =
        resolve_snapshot_plan_type(&state.pool, &latest_row, &snapshot).await?;
    persist_usage_snapshot(
        &state.pool,
        latest_row.id,
        effective_snapshot_plan_type.as_deref(),
        &snapshot,
        state.config.upstream_accounts_history_retention_days,
    )
    .await?;
    let latest_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared after usage snapshot persisted"))?;
    if route_failure_kind_is_quota_exhausted(latest_row.last_route_failure_kind.as_deref())
        && imported_snapshot_is_exhausted(&snapshot)
    {
        record_account_sync_recovery_blocked(
            &state.pool,
            row.id,
            sync_source,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
            "latest usage snapshot still shows an exhausted upstream usage limit window",
            latest_row
                .last_error
                .as_deref()
                .or(row.last_error.as_deref()),
            latest_row.last_route_failure_kind.as_deref(),
        )
        .await?;
        return Ok(());
    }
    if imported_snapshot_is_exhausted(&snapshot) {
        record_account_sync_hard_unavailable(
            &state.pool,
            row.id,
            sync_source,
            UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
            "latest usage snapshot already shows an exhausted upstream usage limit window",
            PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED,
        )
        .await?;
        return Ok(());
    }
    mark_account_sync_success(
        &state.pool,
        row.id,
        sync_source,
        if should_clear_route_failure_state_after_sync_success(&latest_row) {
            SyncSuccessRouteState::ClearFailureState
        } else {
            SyncSuccessRouteState::PreserveFailureState
        },
    )
    .await?;
    Ok(())
}

async fn persist_oauth_credentials(
    pool: &Pool<Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
) -> Result<()> {
    let claims = parse_chatgpt_jwt_claims(&credentials.id_token).unwrap_or_default();
    let encrypted =
        encrypt_credentials(crypto_key, &StoredCredentials::Oauth(credentials.clone()))?;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET encrypted_credentials = ?2,
            token_expires_at = ?3,
            last_refreshed_at = ?4,
            email = COALESCE(?5, email),
            chatgpt_account_id = COALESCE(?6, chatgpt_account_id),
            chatgpt_user_id = COALESCE(?7, chatgpt_user_id),
            plan_type = COALESCE(?8, plan_type),
            plan_type_observed_at = CASE
                WHEN NULLIF(TRIM(?8), '') IS NOT NULL THEN ?4
                ELSE plan_type_observed_at
            END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(encrypted)
    .bind(token_expires_at)
    .bind(&now_iso)
    .bind(claims.email)
    .bind(claims.chatgpt_account_id)
    .bind(claims.chatgpt_user_id)
    .bind(claims.chatgpt_plan_type)
    .execute(pool)
    .await?;
    Ok(())
}

async fn persist_usage_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    effective_plan_type: Option<&str>,
    snapshot: &NormalizedUsageSnapshot,
    retention_days: u64,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .bind(&snapshot.limit_id)
    .bind(&snapshot.limit_name)
    .bind(
        snapshot
            .plan_type
            .clone()
            .or_else(|| effective_plan_type.map(str::to_string)),
    )
    .bind(snapshot.primary.as_ref().map(|value| value.used_percent))
    .bind(
        snapshot
            .primary
            .as_ref()
            .map(|value| value.window_duration_mins),
    )
    .bind(
        snapshot
            .primary
            .as_ref()
            .and_then(|value| value.resets_at.clone()),
    )
    .bind(snapshot.secondary.as_ref().map(|value| value.used_percent))
    .bind(
        snapshot
            .secondary
            .as_ref()
            .map(|value| value.window_duration_mins),
    )
    .bind(
        snapshot
            .secondary
            .as_ref()
            .and_then(|value| value.resets_at.clone()),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .map(|value| if value.has_credits { 1 } else { 0 }),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .map(|value| if value.unlimited { 1 } else { 0 }),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .and_then(|value| value.balance.clone()),
    )
    .execute(pool)
    .await?;

    let retention_cutoff = format_utc_iso(Utc::now() - ChronoDuration::days(retention_days as i64));
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1 AND captured_at < ?2
        "#,
    )
    .bind(account_id)
    .bind(retention_cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

async fn apply_imported_oauth_probe_result(
    state: &AppState,
    account_id: i64,
    probe: &ImportedOauthProbeOutcome,
) -> Result<Option<String>> {
    if let Some(snapshot) = probe.usage_snapshot.as_ref() {
        persist_usage_snapshot(
            &state.pool,
            account_id,
            probe.claims.chatgpt_plan_type.as_deref(),
            snapshot,
            state.config.upstream_accounts_history_retention_days,
        )
        .await?;
        mark_account_sync_success(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT,
            SyncSuccessRouteState::ClearFailureState,
        )
        .await?;
    }
    Ok(probe.usage_snapshot_warning.clone())
}

async fn persist_imported_oauth_existing_inner(
    state: &AppState,
    account_id: i64,
    probe: ImportedOauthProbeOutcome,
) -> Result<Option<String>, (StatusCode, String)> {
    let existing_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
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
    ensure_display_name_available(&mut *tx, &existing_row.display_name, Some(existing_row.id))
        .await?;
    upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(existing_row.id),
            display_name: &existing_row.display_name,
            group_name: existing_row.group_name.clone(),
            is_mother: existing_row.is_mother != 0,
            note: existing_row.note.clone(),
            tag_ids: existing_tag_ids,
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &probe.claims,
            encrypted_credentials,
            token_expires_at: &probe.token_expires_at,
            external_identity: None,
        },
    )
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

    match apply_imported_oauth_probe_result(state, account_id, &probe).await {
        Ok(warning) => Ok(warning),
        Err(err) => {
            warn!(
                account_id,
                error = %err,
                "imported OAuth credential persisted but post-import state update failed"
            );
            Ok(Some(format!(
                "Imported, but post-import state update failed: {err}"
            )))
        }
    }
}

struct OauthAccountUpsert<'a> {
    account_id: Option<i64>,
    display_name: &'a str,
    group_name: Option<String>,
    is_mother: bool,
    note: Option<String>,
    tag_ids: Vec<i64>,
    requested_group_metadata_changes: RequestedGroupMetadataChanges,
    claims: &'a ChatgptJwtClaims,
    encrypted_credentials: String,
    token_expires_at: &'a str,
    external_identity: Option<&'a ExternalAccountIdentity>,
}

fn duplicate_display_name_error() -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        "displayName must be unique".to_string(),
    )
}

async fn load_conflicting_display_name_id(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    display_name: &str,
    exclude_id: Option<i64>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE lower(trim(display_name)) = lower(trim(?1))
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

async fn ensure_display_name_available(
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

async fn upsert_oauth_account(
    tx: &mut Transaction<'_, Sqlite>,
    payload: OauthAccountUpsert<'_>,
) -> Result<i64> {
    let OauthAccountUpsert {
        account_id,
        display_name,
        group_name,
        is_mother,
        note,
        tag_ids,
        requested_group_metadata_changes,
        claims,
        encrypted_credentials,
        token_expires_at,
        external_identity,
    } = payload;
    let target_group_name = group_name.clone();
    let now_iso = format_utc_iso(Utc::now());
    let resolved_account_id = account_id;
    let external_client_id = external_identity.map(|value| value.client_id.as_str());
    let external_source_account_id = external_identity.map(|value| value.source_account_id.as_str());

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
                chatgpt_account_id = ?10,
                chatgpt_user_id = ?11,
                plan_type = ?12,
                plan_type_observed_at = CASE
                    WHEN NULLIF(TRIM(?12), '') IS NOT NULL THEN ?15
                    ELSE plan_type_observed_at
                END,
                encrypted_credentials = ?13,
                token_expires_at = ?14,
                last_refreshed_at = ?15,
                external_client_id = COALESCE(?16, external_client_id),
                external_source_account_id = COALESCE(?17, external_source_account_id),
                last_error = NULL,
                last_error_at = NULL,
                updated_at = ?15
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
        .bind(claims.email.clone())
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .bind(external_client_id)
        .bind(external_source_account_id)
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
                email, chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at,
                masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at,
                last_error, last_error_at, local_primary_limit, local_secondary_limit,
                local_limit_unit, external_client_id, external_source_account_id,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 1,
                ?8, ?9, ?10, ?11, ?12,
                NULL, ?13, ?14,
                ?15, NULL, NULL,
                NULL, NULL, NULL, NULL,
                NULL, ?16, ?17, ?15, ?15
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
        .bind(claims.email.clone())
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
struct UpstreamAccountIdentityRow {
    id: i64,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
}

#[derive(Debug, Clone)]
struct UpstreamAccountIdentityClusterMember {
    id: i64,
    chatgpt_user_id: Option<String>,
    group_name: Option<String>,
    plan_type: Option<String>,
}

fn normalize_plan_type(plan_type: Option<&str>) -> Option<String> {
    plan_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_team_plan_type(plan_type: Option<&str>) -> bool {
    normalize_plan_type(plan_type)
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("team"))
}

fn should_flag_duplicate_identity_pair(
    current_plan_type: Option<&str>,
    peer_plan_type: Option<&str>,
) -> bool {
    match (
        normalize_plan_type(current_plan_type),
        normalize_plan_type(peer_plan_type),
    ) {
        (Some(current), Some(peer)) => current.eq_ignore_ascii_case(&peer),
        _ => true,
    }
}

fn is_team_shared_org_cluster_member(member: &UpstreamAccountIdentityClusterMember) -> bool {
    is_team_plan_type(member.plan_type.as_deref())
        && member.group_name.is_some()
        && member.chatgpt_user_id.is_some()
}

fn is_team_shared_org_peer_pair(
    current: &UpstreamAccountIdentityClusterMember,
    peer: &UpstreamAccountIdentityClusterMember,
) -> bool {
    is_team_shared_org_cluster_member(current)
        && is_team_shared_org_cluster_member(peer)
        && current.group_name == peer.group_name
        && current.chatgpt_user_id != peer.chatgpt_user_id
}

fn resolve_effective_plan_type(
    account_plan_type: Option<&str>,
    sample_plan_type: Option<&str>,
) -> Option<String> {
    normalize_plan_type(sample_plan_type).or_else(|| normalize_plan_type(account_plan_type))
}

async fn resolve_snapshot_plan_type(
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

async fn load_duplicate_info_map(
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
                            ORDER BY previous_sample.captured_at DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = account.id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC
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
                    group_name: normalize_optional_text(row.group_name.clone()),
                    plan_type: normalize_plan_type(row.plan_type.as_deref()),
                },
            );
        }
        if let Some(chatgpt_user_id) = row.chatgpt_user_id.as_ref().cloned() {
            by_user_id.entry(chatgpt_user_id).or_default().push(
                UpstreamAccountIdentityClusterMember {
                    id: row.id,
                    chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
                    group_name: normalize_optional_text(row.group_name.clone()),
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
            group_name: normalize_optional_text(row.group_name.clone()),
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
                    && !is_team_shared_org_peer_pair(&current_member, member)
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
                    && !is_team_shared_org_peer_pair(&current_member, member)
                    && should_flag_duplicate_identity_pair(
                        current_member.plan_type.as_deref(),
                        member.plan_type.as_deref(),
                    )
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

async fn load_account_tag_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, Vec<AccountTagSummary>>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            link.account_id,
            tag.id AS tag_id,
            tag.name,
            tag.guard_enabled,
            tag.lookback_hours,
            tag.max_conversations,
            tag.allow_cut_out,
            tag.allow_cut_in,
            tag.priority_tier,
            tag.fast_mode_rewrite_mode,
            tag.concurrency_limit
        FROM pool_upstream_account_tags link
        INNER JOIN pool_tags tag ON tag.id = link.tag_id
        WHERE link.account_id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(") ORDER BY tag.name COLLATE NOCASE ASC, tag.id ASC")
        .build_query_as::<AccountTagRow>()
        .fetch_all(pool)
        .await?;
    let mut grouped: HashMap<i64, Vec<AccountTagSummary>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.account_id)
            .or_default()
            .push(account_tag_summary_from_row(&row));
    }
    Ok(grouped)
}

async fn load_tags_by_ids(pool: &Pool<Sqlite>, tag_ids: &[i64]) -> Result<Vec<TagRow>> {
    if tag_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            name,
            guard_enabled,
            lookback_hours,
            max_conversations,
            allow_cut_out,
            allow_cut_in,
            priority_tier,
            fast_mode_rewrite_mode,
            concurrency_limit
        FROM pool_tags
        WHERE id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for tag_id in tag_ids {
            separated.push_bind(tag_id);
        }
    }
    query
        .push(") ORDER BY name COLLATE NOCASE ASC, id ASC")
        .build_query_as::<TagRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn load_tag_row(pool: &Pool<Sqlite>, tag_id: i64) -> Result<Option<TagRow>> {
    sqlx::query_as::<_, TagRow>(
        r#"
        SELECT
            name,
            guard_enabled,
            lookback_hours,
            max_conversations,
            allow_cut_out,
            allow_cut_in,
            priority_tier,
            fast_mode_rewrite_mode,
            concurrency_limit
        FROM pool_tags
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(tag_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_tag_detail(pool: &Pool<Sqlite>, tag_id: i64) -> Result<Option<TagDetail>> {
    let items = load_tag_summaries(
        pool,
        &ListTagsQuery {
            search: None,
            has_accounts: None,
            guard_enabled: None,
            allow_cut_in: None,
            allow_cut_out: None,
        },
    )
    .await?;
    Ok(items
        .into_iter()
        .find(|item| item.id == tag_id)
        .map(|summary| TagDetail { summary }))
}

async fn load_tag_summaries(
    pool: &Pool<Sqlite>,
    params: &ListTagsQuery,
) -> Result<Vec<TagSummary>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            tag.id,
            tag.name,
            tag.guard_enabled,
            tag.lookback_hours,
            tag.max_conversations,
            tag.allow_cut_out,
            tag.allow_cut_in,
            tag.priority_tier,
            tag.fast_mode_rewrite_mode,
            tag.concurrency_limit,
            tag.updated_at,
            COUNT(DISTINCT link.account_id) AS account_count,
            COUNT(DISTINCT NULLIF(TRIM(account.group_name), '')) AS group_count
        FROM pool_tags tag
        LEFT JOIN pool_upstream_account_tags link ON link.tag_id = tag.id
        LEFT JOIN pool_upstream_accounts account ON account.id = link.account_id
        WHERE 1 = 1
        "#,
    );
    if let Some(search) = params
        .search
        .as_ref()
        .and_then(|value| normalize_optional_text(Some(value.clone())))
    {
        query
            .push(" AND tag.name LIKE ")
            .push_bind(format!("%{search}%"));
    }
    if let Some(guard_enabled) = params.guard_enabled {
        query
            .push(" AND tag.guard_enabled = ")
            .push_bind(if guard_enabled { 1 } else { 0 });
    }
    if let Some(allow_cut_in) = params.allow_cut_in {
        query
            .push(" AND tag.allow_cut_in = ")
            .push_bind(if allow_cut_in { 1 } else { 0 });
    }
    if let Some(allow_cut_out) = params.allow_cut_out {
        query
            .push(" AND tag.allow_cut_out = ")
            .push_bind(if allow_cut_out { 1 } else { 0 });
    }
    query.push(
        " GROUP BY tag.id, tag.name, tag.guard_enabled, tag.lookback_hours, tag.max_conversations, tag.allow_cut_out, tag.allow_cut_in, tag.priority_tier, tag.fast_mode_rewrite_mode, tag.concurrency_limit, tag.updated_at",
    );
    if let Some(has_accounts) = params.has_accounts {
        query.push(if has_accounts {
            " HAVING COUNT(DISTINCT link.account_id) > 0"
        } else {
            " HAVING COUNT(DISTINCT link.account_id) = 0"
        });
    }
    let rows = query
        .push(" ORDER BY tag.updated_at DESC, tag.id DESC")
        .build_query_as::<TagListRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| tag_summary_from_row(&row))
        .collect())
}

async fn insert_tag(pool: &Pool<Sqlite>, name: &str, rule: &TagRoutingRule) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations, allow_cut_out, allow_cut_in, priority_tier, fast_mode_rewrite_mode, concurrency_limit, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(if rule.guard_enabled { 1 } else { 0 })
    .bind(rule.lookback_hours)
    .bind(rule.max_conversations)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(rule.priority_tier.as_str())
    .bind(rule.fast_mode_rewrite_mode.as_str())
    .bind(rule.concurrency_limit)
    .bind(&now_iso)
    .fetch_one(pool)
    .await?;
    load_tag_detail(pool, inserted_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after insert"))
}

async fn persist_tag_update(
    pool: &Pool<Sqlite>,
    tag_id: i64,
    name: &str,
    rule: &TagRoutingRule,
) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_tags
        SET name = ?2,
            guard_enabled = ?3,
            lookback_hours = ?4,
            max_conversations = ?5,
            allow_cut_out = ?6,
            allow_cut_in = ?7,
            priority_tier = ?8,
            fast_mode_rewrite_mode = ?9,
            concurrency_limit = ?10,
            updated_at = ?11
        WHERE id = ?1
        "#,
    )
    .bind(tag_id)
    .bind(name)
    .bind(if rule.guard_enabled { 1 } else { 0 })
    .bind(rule.lookback_hours)
    .bind(rule.max_conversations)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(rule.priority_tier.as_str())
    .bind(rule.fast_mode_rewrite_mode.as_str())
    .bind(rule.concurrency_limit)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    load_tag_detail(pool, tag_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after update"))
}

async fn delete_tag_by_id(pool: &Pool<Sqlite>, tag_id: i64) -> Result<(), (StatusCode, String)> {
    let linked_account_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_account_tags WHERE tag_id = ?1",
    )
    .bind(tag_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error_tuple)?;
    let linked_session_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_oauth_login_sessions
        WHERE tag_ids_json IS NOT NULL
          AND EXISTS (
              SELECT 1
              FROM json_each(pool_oauth_login_sessions.tag_ids_json)
              WHERE CAST(json_each.value AS INTEGER) = ?1
          )
        "#,
    )
    .bind(tag_id)
    .fetch_one(pool)
    .await
    .map_err(internal_error_tuple)?;
    if linked_account_count > 0 || linked_session_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            "tag is still associated with accounts or pending OAuth sessions".to_string(),
        ));
    }
    let affected = sqlx::query("DELETE FROM pool_tags WHERE id = ?1")
        .bind(tag_id)
        .execute(pool)
        .await
        .map_err(internal_error_tuple)?
        .rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "tag not found".to_string()));
    }
    Ok(())
}

fn map_tag_write_error(err: anyhow::Error) -> (StatusCode, String) {
    let message = err.to_string();
    if message.contains("UNIQUE constraint failed") {
        (StatusCode::CONFLICT, "tag name already exists".to_string())
    } else {
        internal_error_tuple(err)
    }
}

async fn validate_tag_ids(
    pool: &Pool<Sqlite>,
    tag_ids: &[i64],
) -> Result<Vec<i64>, (StatusCode, String)> {
    let mut normalized = tag_ids
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        return Ok(normalized);
    }
    let rows = load_tags_by_ids(pool, &normalized)
        .await
        .map_err(internal_error_tuple)?;
    if rows.len() != normalized.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            "one or more tagIds do not exist".to_string(),
        ));
    }
    Ok(normalized)
}

async fn sync_account_tag_links_with_executor(
    conn: &mut SqliteConnection,
    account_id: i64,
    tag_ids: &[i64],
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query("DELETE FROM pool_upstream_account_tags WHERE account_id = ?1")
        .bind(account_id)
        .execute(&mut *conn)
        .await?;
    for tag_id in tag_ids {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_tags (
                account_id, tag_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?3)
            "#,
        )
        .bind(account_id)
        .bind(tag_id)
        .bind(&now_iso)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn sync_account_tag_links(
    pool: &Pool<Sqlite>,
    account_id: i64,
    tag_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sync_account_tag_links_with_executor(&mut *tx, account_id, tag_ids).await?;
    tx.commit().await?;
    Ok(())
}

async fn count_recent_account_conversations(
    pool: &Pool<Sqlite>,
    account_id: i64,
    lookback_hours: i64,
) -> Result<i64> {
    let lower_bound = format_utc_iso(Utc::now() - ChronoDuration::hours(lookback_hours));
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_sticky_routes
        WHERE account_id = ?1
          AND last_seen_at >= ?2
        "#,
    )
    .bind(account_id)
    .bind(lower_bound)
    .fetch_one(pool)
    .await
    .map_err(Into::into)
}

async fn load_upstream_account_groups(
    pool: &Pool<Sqlite>,
) -> Result<Vec<UpstreamAccountGroupSummary>> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        ),
    >(
        r#"
        SELECT
            groups.group_name,
            notes.note,
            notes.bound_proxy_keys_json,
            notes.node_shunt_enabled,
            notes.upstream_429_retry_enabled,
            notes.upstream_429_max_retries,
            notes.concurrency_limit
        FROM (
            SELECT DISTINCT TRIM(group_name) AS group_name
            FROM pool_upstream_accounts
            WHERE group_name IS NOT NULL AND TRIM(group_name) <> ''
        ) groups
        LEFT JOIN pool_upstream_account_group_notes notes
            ON notes.group_name = groups.group_name
        ORDER BY groups.group_name COLLATE NOCASE ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                group_name,
                note,
                bound_proxy_keys_json,
                node_shunt_enabled,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
                concurrency_limit,
            )| {
                let node_shunt_enabled =
                    decode_group_node_shunt_enabled(node_shunt_enabled.unwrap_or_default());
                let upstream_429_retry_enabled = decode_group_upstream_429_retry_enabled(
                    upstream_429_retry_enabled.unwrap_or_default(),
                );
                let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
                    upstream_429_retry_enabled,
                    decode_group_upstream_429_max_retries(
                        upstream_429_max_retries.unwrap_or_default(),
                    ),
                );
                UpstreamAccountGroupSummary {
                    group_name,
                    note: normalize_optional_text(note),
                    bound_proxy_keys: decode_group_bound_proxy_keys_json(
                        bound_proxy_keys_json.as_deref(),
                    ),
                    node_shunt_enabled,
                    upstream_429_retry_enabled,
                    upstream_429_max_retries,
                    concurrency_limit: concurrency_limit.unwrap_or_default(),
                }
            },
        )
        .collect())
}
async fn load_upstream_account_summaries(
    pool: &Pool<Sqlite>,
) -> Result<Vec<UpstreamAccountSummary>> {
    load_upstream_account_summaries_for_query(pool, &ListUpstreamAccountsQuery::default()).await
}

async fn load_upstream_account_summaries_for_query(
    pool: &Pool<Sqlite>,
    params: &ListUpstreamAccountsQuery,
) -> Result<Vec<UpstreamAccountSummary>> {
    let duplicate_info_map = load_duplicate_info_map(pool).await?;
    let mut normalized_tag_ids = params
        .tag_ids
        .iter()
        .copied()
        .filter(|tag_id| *tag_id > 0)
        .collect::<Vec<_>>();
    normalized_tag_ids.sort_unstable();
    normalized_tag_ids.dedup();
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            compact_support_status, compact_support_observed_at,
            compact_support_reason, local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, created_at, updated_at
        FROM pool_upstream_accounts
        "#,
    );
    query.push(" WHERE 1 = 1");

    if params.group_ungrouped.unwrap_or(false) {
        query.push(" AND NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL");
    } else if let Some(group_search) = params
        .group_search
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        query
            .push(" AND LOWER(TRIM(COALESCE(group_name, ''))) LIKE ")
            .push_bind(format!("%{}%", group_search.to_lowercase()));
    }

    if !normalized_tag_ids.is_empty() {
        query.push(" AND id IN (SELECT link.account_id FROM pool_upstream_account_tags link WHERE link.tag_id IN (");
        {
            let mut separated = query.separated(", ");
            for tag_id in &normalized_tag_ids {
                separated.push_bind(tag_id);
            }
        }
        query
            .push(") GROUP BY link.account_id HAVING COUNT(DISTINCT link.tag_id) = ")
            .push_bind(normalized_tag_ids.len() as i64)
            .push(")");
    }

    let rows = query
        .push(" ORDER BY updated_at DESC, id DESC")
        .build_query_as::<UpstreamAccountRow>()
        .fetch_all(pool)
        .await?;
    let now = Utc::now();
    let account_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let tag_map = load_account_tag_map(pool, &account_ids).await?;
    let active_conversation_count_map =
        load_account_active_conversation_count_map(pool, &account_ids, now.clone()).await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let latest = load_latest_usage_sample(pool, row.id).await?;
        let tags = tag_map.get(&row.id).cloned().unwrap_or_default();
        items.push(build_summary_from_row(
            &row,
            latest.as_ref(),
            row.last_activity_at.clone(),
            tags,
            duplicate_info_map.get(&row.id).cloned(),
            active_conversation_count_map
                .get(&row.id)
                .copied()
                .unwrap_or_default(),
            now.clone(),
        ));
    }
    Ok(items)
}

async fn build_bulk_upstream_account_sync_pending_rows(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<Vec<BulkUpstreamAccountSyncRow>> {
    let mut rows = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        let display_name = load_upstream_account_row(pool, *account_id)
            .await?
            .map(|row| row.display_name)
            .unwrap_or_else(|| format!("Account {account_id}"));
        rows.push(BulkUpstreamAccountSyncRow {
            account_id: *account_id,
            display_name,
            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING.to_string(),
            detail: None,
        });
    }
    Ok(rows)
}

async fn apply_bulk_upstream_account_action(
    state: Arc<AppState>,
    account_id: i64,
    action: &str,
    group_name: Option<String>,
    tag_ids: Vec<i64>,
) -> Result<(), (StatusCode, String)> {
    let payload = match action {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE => UpdateUpstreamAccountRequest {
            display_name: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            enabled: Some(true),
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE => UpdateUpstreamAccountRequest {
            display_name: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            enabled: Some(false),
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP => UpdateUpstreamAccountRequest {
            display_name: None,
            group_name,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            enabled: None,
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
        },
        BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS | BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS => {
            let current_tag_ids = load_account_tag_map(&state.pool, &[account_id])
                .await
                .map_err(internal_error_tuple)?
                .remove(&account_id)
                .unwrap_or_default()
                .into_iter()
                .map(|tag| tag.id)
                .collect::<BTreeSet<_>>();
            let tag_id_set = tag_ids.into_iter().collect::<BTreeSet<_>>();
            let next_tag_ids = if action == BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS {
                current_tag_ids
                    .union(&tag_id_set)
                    .copied()
                    .collect::<Vec<_>>()
            } else {
                current_tag_ids
                    .difference(&tag_id_set)
                    .copied()
                    .collect::<Vec<_>>()
            };
            UpdateUpstreamAccountRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: Some(next_tag_ids),
            }
        }
        BULK_UPSTREAM_ACCOUNT_ACTION_DELETE => {
            state
                .upstream_accounts
                .account_ops
                .run_delete_account(state.clone(), account_id)
                .await?;
            return Ok(());
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "unsupported bulk action".to_string(),
            ));
        }
    };

    state
        .upstream_accounts
        .account_ops
        .run_update_account(state.clone(), account_id, payload)
        .await?;
    Ok(())
}

async fn has_ungrouped_upstream_accounts(pool: &Pool<Sqlite>) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE NULLIF(TRIM(COALESCE(group_name, '')), '') IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

async fn load_upstream_account_detail(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let Some(row) = load_upstream_account_row(pool, id).await? else {
        return Ok(None);
    };
    let latest = load_latest_usage_sample(pool, row.id).await?;
    let tags = load_account_tag_map(pool, &[row.id])
        .await?
        .remove(&row.id)
        .unwrap_or_default();
    let history_rows = sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
        ORDER BY captured_at DESC
        LIMIT 128
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let mut history = history_rows
        .into_iter()
        .map(|sample| UpstreamAccountHistoryPoint {
            captured_at: sample.captured_at,
            primary_used_percent: sample.primary_used_percent,
            secondary_used_percent: sample.secondary_used_percent,
            credits_balance: sample.credits_balance,
        })
        .collect::<Vec<_>>();
    history.reverse();
    let recent_action_rows = sqlx::query_as::<_, UpstreamAccountActionEventRow>(
        r#"
        SELECT
            id, occurred_at, action, source, reason_code, reason_message,
            http_status, failure_kind, invoke_id, sticky_key, created_at
        FROM pool_upstream_account_events
        WHERE account_id = ?1
        ORDER BY occurred_at DESC, id DESC
        LIMIT 20
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let duplicate_info_map = load_duplicate_info_map(pool).await?;
    let now = Utc::now();
    let active_conversation_count =
        load_account_active_conversation_count_map(pool, &[row.id], now.clone())
            .await?
            .get(&row.id)
            .copied()
            .unwrap_or_default();
    let summary = build_summary_from_row(
        &row,
        latest.as_ref(),
        row.last_activity_at.clone(),
        tags,
        duplicate_info_map.get(&row.id).cloned(),
        active_conversation_count,
        now,
    );
    Ok(Some(UpstreamAccountDetail {
        summary,
        note: row.note,
        upstream_base_url: row.upstream_base_url,
        chatgpt_user_id: row.chatgpt_user_id,
        last_refreshed_at: row.last_refreshed_at,
        history,
        recent_actions: recent_action_rows
            .iter()
            .map(build_action_event_from_row)
            .collect(),
    }))
}

async fn load_upstream_account_detail_with_actual_usage(
    state: &AppState,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let mut detail = match load_upstream_account_detail(&state.pool, id).await? {
        Some(detail) => detail,
        None => return Ok(None),
    };
    let groups = load_canonicalized_upstream_account_groups(state).await?;
    enrich_window_actual_usage_for_summaries(state, std::slice::from_mut(&mut detail.summary))
        .await?;
    enrich_node_shunt_routing_block_reasons(state, std::slice::from_mut(&mut detail.summary))
        .await?;
    enrich_current_forward_proxy_for_summaries(
        state,
        &groups,
        std::slice::from_mut(&mut detail.summary),
    )
    .await?;
    Ok(Some(detail))
}

async fn load_upstream_account_row(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    let mut conn = pool.acquire().await?;
    load_upstream_account_row_conn(&mut conn, id).await
}

async fn load_upstream_account_row_conn(
    conn: &mut SqliteConnection,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            compact_support_status, compact_support_observed_at,
            compact_support_reason, local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, external_client_id,
            external_source_account_id, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(conn)
    .await
    .map_err(Into::into)
}

async fn load_upstream_account_row_by_external_identity(
    pool: &Pool<Sqlite>,
    external_client_id: &str,
    external_source_account_id: &str,
) -> Result<Option<UpstreamAccountRow>> {
    let mut conn = pool.acquire().await?;
    load_upstream_account_row_by_external_identity_conn(
        &mut conn,
        external_client_id,
        external_source_account_id,
    )
    .await
}

async fn load_upstream_account_row_by_external_identity_conn(
    conn: &mut SqliteConnection,
    external_client_id: &str,
    external_source_account_id: &str,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, is_mother, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
            last_action, last_action_source, last_action_reason_code, last_action_reason_message,
            last_action_http_status, last_action_invoke_id, last_action_at,
            last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
            consecutive_route_failures, temporary_route_failure_streak_started_at,
            compact_support_status, compact_support_observed_at,
            compact_support_reason, local_primary_limit, local_secondary_limit,
            local_limit_unit, upstream_base_url, external_client_id,
            external_source_account_id, created_at, updated_at
        FROM pool_upstream_accounts
        WHERE external_client_id = ?1
          AND external_source_account_id = ?2
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(external_client_id)
    .bind(external_source_account_id)
    .fetch_optional(conn)
    .await
    .map_err(Into::into)
}

async fn load_latest_usage_sample(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<UpstreamAccountSampleRow>> {
    sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            sample.captured_at,
            sample.limit_id,
            sample.limit_name,
            COALESCE(
                CASE
                    WHEN NULLIF(TRIM(account.plan_type), '') IS NOT NULL
                         AND account.plan_type_observed_at IS NOT NULL
                         AND julianday(account.plan_type_observed_at) >= julianday((
                            SELECT previous_sample.captured_at
                            FROM pool_upstream_account_limit_samples previous_sample
                            WHERE previous_sample.account_id = sample.account_id
                              AND previous_sample.plan_type IS NOT NULL
                              AND TRIM(previous_sample.plan_type) <> ''
                            ORDER BY previous_sample.captured_at DESC
                            LIMIT 1
                         ))
                        THEN NULLIF(TRIM(account.plan_type), '')
                    ELSE (
                        SELECT NULLIF(TRIM(previous_sample.plan_type), '')
                        FROM pool_upstream_account_limit_samples previous_sample
                        WHERE previous_sample.account_id = sample.account_id
                          AND previous_sample.plan_type IS NOT NULL
                          AND TRIM(previous_sample.plan_type) <> ''
                        ORDER BY previous_sample.captured_at DESC
                        LIMIT 1
                    )
                END,
                NULLIF(TRIM(account.plan_type), '')
            ) AS plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        FROM pool_upstream_account_limit_samples sample
        INNER JOIN pool_upstream_accounts account ON account.id = sample.account_id
        WHERE sample.account_id = ?1
        ORDER BY sample.captured_at DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn build_summary_from_row(
    row: &UpstreamAccountRow,
    sample: Option<&UpstreamAccountSampleRow>,
    last_activity_at: Option<String>,
    tags: Vec<AccountTagSummary>,
    duplicate_info: Option<DuplicateInfo>,
    active_conversation_count: i64,
    now: DateTime<Utc>,
) -> UpstreamAccountSummary {
    let local_limits = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        Some(LocalLimitSnapshot {
            primary_limit: row.local_primary_limit,
            secondary_limit: row.local_secondary_limit,
            limit_unit: row
                .local_limit_unit
                .clone()
                .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string()),
        })
    } else {
        None
    };
    let primary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_primary_limit,
            row.local_limit_unit.as_deref(),
            300,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.primary_used_percent,
                value.primary_window_minutes,
                value.primary_resets_at.as_deref(),
            )
        })
    };
    let secondary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_secondary_limit,
            row.local_limit_unit.as_deref(),
            10_080,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.secondary_used_percent,
                value.secondary_window_minutes,
                value.secondary_resets_at.as_deref(),
            )
        })
    };
    let credits = sample.and_then(|value| {
        value
            .credits_has_credits
            .map(|has_credits| CreditsSnapshot {
                has_credits: has_credits != 0,
                unlimited: value.credits_unlimited.unwrap_or_default() != 0,
                balance: value.credits_balance.clone(),
            })
    });
    let effective_routing_rule = build_effective_routing_rule(&tags);
    let status = effective_account_status(row);
    let enable_status = derive_upstream_account_enable_status(row.enabled != 0);
    let health_status = derive_upstream_account_health_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    );
    let sync_state = derive_upstream_account_sync_state(row.enabled != 0, &row.status);
    let snapshot_exhausted = persisted_usage_sample_is_exhausted(sample);
    let work_status = derive_upstream_account_work_status(
        row.enabled != 0,
        &row.status,
        health_status,
        sync_state,
        snapshot_exhausted,
        row.cooldown_until.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
        row.temporary_route_failure_streak_started_at.as_deref(),
        row.last_selected_at.as_deref(),
        now,
    );
    let display_status = classify_upstream_account_display_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    )
    .to_string();
    let compact_support = build_compact_support_state(row);

    UpstreamAccountSummary {
        id: row.id,
        kind: row.kind.clone(),
        provider: row.provider.clone(),
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        is_mother: row.is_mother != 0,
        status,
        display_status,
        enabled: row.enabled != 0,
        work_status: work_status.to_string(),
        enable_status: enable_status.to_string(),
        health_status: health_status.to_string(),
        sync_state: sync_state.to_string(),
        email: row.email.clone(),
        chatgpt_account_id: row.chatgpt_account_id.clone(),
        plan_type: resolve_effective_plan_type(
            row.plan_type.as_deref(),
            sample.and_then(|value| value.plan_type.as_deref()),
        ),
        masked_api_key: row.masked_api_key.clone(),
        last_synced_at: row.last_synced_at.clone(),
        last_successful_sync_at: row.last_successful_sync_at.clone(),
        last_activity_at: last_activity_at
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .map(format_utc_iso)
            .or(last_activity_at),
        active_conversation_count,
        last_error: row.last_error.clone(),
        last_error_at: row.last_error_at.clone(),
        last_action: row.last_action.clone(),
        last_action_source: row.last_action_source.clone(),
        last_action_reason_code: row.last_action_reason_code.clone(),
        last_action_reason_message: row.last_action_reason_message.clone(),
        last_action_http_status: row
            .last_action_http_status
            .and_then(|value| u16::try_from(value).ok()),
        last_action_invoke_id: row.last_action_invoke_id.clone(),
        last_action_at: row.last_action_at.clone(),
        cooldown_until: row.cooldown_until.clone(),
        current_forward_proxy_key: None,
        current_forward_proxy_display_name: None,
        current_forward_proxy_state: UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED.to_string(),
        routing_block_reason_code: None,
        routing_block_reason_message: None,
        token_expires_at: row.token_expires_at.clone(),
        primary_window,
        secondary_window,
        credits,
        local_limits,
        compact_support,
        duplicate_info,
        tags,
        effective_routing_rule,
    }
}

fn apply_node_shunt_routing_block_reasons_to_summaries(
    items: &mut [UpstreamAccountSummary],
    assignments: &UpstreamAccountNodeShuntAssignments,
) {
    for item in items {
        item.routing_block_reason_code = None;
        item.routing_block_reason_message = None;
        let Some(group_name) = normalize_optional_text(item.group_name.clone()) else {
            continue;
        };
        if !assignments.group_slots.contains_key(&group_name) {
            continue;
        }
        if !assignments.eligible_account_ids.contains(&item.id) {
            continue;
        }
        if assignments.account_proxy_keys.contains_key(&item.id) {
            continue;
        }
        item.work_status = UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string();
        item.health_status = UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string();
        item.routing_block_reason_code =
            Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED.to_string());
        item.routing_block_reason_message =
            Some(group_node_shunt_unassigned_error_message().to_string());
    }
}

async fn enrich_node_shunt_routing_block_reasons(
    state: &AppState,
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let assignments = build_upstream_account_node_shunt_assignments(state).await?;
    apply_node_shunt_routing_block_reasons_to_summaries(items, &assignments);
    Ok(())
}

async fn load_canonicalized_upstream_account_groups(
    state: &AppState,
) -> Result<Vec<UpstreamAccountGroupSummary>> {
    let mut groups = load_upstream_account_groups(&state.pool).await?;
    for group in &mut groups {
        group.bound_proxy_keys =
            canonicalize_forward_proxy_bound_keys(state, &group.bound_proxy_keys).await?;
    }
    Ok(groups)
}

fn assign_current_forward_proxy(
    item: &mut UpstreamAccountSummary,
    state: &'static str,
    proxy_key: Option<String>,
    proxy_display_name: Option<String>,
) {
    item.current_forward_proxy_key = proxy_key;
    item.current_forward_proxy_display_name = proxy_display_name;
    item.current_forward_proxy_state = state.to_string();
}

fn resolve_current_forward_proxy_display_name(
    manager: &crate::forward_proxy::ForwardProxyManager,
    binding_display_names: &HashMap<String, String>,
    metadata_map: &HashMap<String, crate::forward_proxy::ForwardProxyMetadataHistoryRow>,
    binding_key: &str,
) -> Option<String> {
    binding_display_names.get(binding_key).cloned().or_else(|| {
        let history_row = metadata_map.get(binding_key);
        manager
            .resolve_current_or_historical_bound_proxy_key(binding_key, history_row)
            .and_then(|resolved_key| binding_display_names.get(&resolved_key).cloned())
            .or_else(|| {
                history_row
                    .map(|row| row.display_name.trim())
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
    })
}

async fn enrich_current_forward_proxy_for_summaries(
    state: &AppState,
    groups: &[UpstreamAccountGroupSummary],
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }

    let group_map = groups
        .iter()
        .cloned()
        .map(|group| (group.group_name.clone(), group))
        .collect::<HashMap<_, _>>();
    let node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    let (binding_display_names, shared_group_current_bindings) = {
        let manager = state.forward_proxy.lock().await;
        let binding_display_names = manager
            .binding_nodes()
            .into_iter()
            .map(|node| (node.key, node.display_name))
            .collect::<HashMap<_, _>>();
        let shared_group_current_bindings = groups
            .iter()
            .filter(|group| !group.node_shunt_enabled)
            .filter_map(|group| {
                manager
                    .current_bound_group_binding_key(&group.group_name, &group.bound_proxy_keys)
                    .map(|binding_key| (group.group_name.clone(), binding_key))
            })
            .collect::<HashMap<_, _>>();
        (binding_display_names, shared_group_current_bindings)
    };
    let relevant_binding_keys = shared_group_current_bindings
        .values()
        .cloned()
        .chain(node_shunt_assignments.account_proxy_keys.values().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let metadata_map = crate::forward_proxy::load_forward_proxy_metadata_history(
        &state.pool,
        &relevant_binding_keys,
    )
    .await?;
    let recovered_binding_display_names = {
        let manager = state.forward_proxy.lock().await;
        relevant_binding_keys
            .into_iter()
            .filter_map(|binding_key| {
                resolve_current_forward_proxy_display_name(
                    &manager,
                    &binding_display_names,
                    &metadata_map,
                    &binding_key,
                )
                .map(|display_name| (binding_key, display_name))
            })
            .collect::<HashMap<_, _>>()
    };

    for item in items {
        assign_current_forward_proxy(
            item,
            UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED,
            None,
            None,
        );
        let Some(group_name) = normalize_optional_text(item.group_name.clone()) else {
            continue;
        };
        let Some(group) = group_map.get(&group_name) else {
            continue;
        };

        if group.node_shunt_enabled {
            if let Some(binding_key) = node_shunt_assignments.account_proxy_keys.get(&item.id) {
                assign_current_forward_proxy(
                    item,
                    UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
                    Some(binding_key.clone()),
                    recovered_binding_display_names.get(binding_key).cloned(),
                );
            } else if node_shunt_assignments.eligible_account_ids.contains(&item.id)
                && node_shunt_assignments
                    .group_slots
                    .get(&group_name)
                    .is_some_and(|slots| !slots.valid_proxy_keys.is_empty())
            {
                assign_current_forward_proxy(
                    item,
                    UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_PENDING,
                    None,
                    None,
                );
            }
            continue;
        }

        let Some(binding_key) = shared_group_current_bindings.get(&group_name).cloned() else {
            continue;
        };
        assign_current_forward_proxy(
            item,
            UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED,
            Some(binding_key.clone()),
            recovered_binding_display_names.get(&binding_key).cloned(),
        );
    }

    Ok(())
}

fn collect_forward_proxy_catalog_keys(
    groups: &[UpstreamAccountGroupSummary],
    items: &[UpstreamAccountSummary],
) -> Vec<String> {
    let relevant_group_names = items
        .iter()
        .filter_map(|item| normalize_optional_text(item.group_name.clone()))
        .collect::<HashSet<_>>();
    groups
        .iter()
        .filter(|group| relevant_group_names.contains(&group.group_name))
        .flat_map(|group| group.bound_proxy_keys.iter().cloned())
        .chain(
            items.iter()
                .filter_map(|item| item.current_forward_proxy_key.clone()),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

async fn enrich_window_actual_usage_for_summaries(
    state: &AppState,
    items: &mut [UpstreamAccountSummary],
) -> Result<()> {
    if items.is_empty() || !sqlite_table_exists(&state.pool, "codex_invocations").await? {
        return Ok(());
    }

    let now = Utc::now();
    let Some((plans, query_start, query_end)) = collect_account_window_usage_plans(items, now)
    else {
        return Ok(());
    };
    let account_ids = plans.keys().copied().collect::<Vec<_>>();
    if account_ids.is_empty() {
        return Ok(());
    }

    let query_start_at = format_naive(query_start.with_timezone(&Shanghai).naive_local());
    let query_end_at = format_naive(query_end.with_timezone(&Shanghai).naive_local());
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let mut rows = Vec::new();

    let live_start = query_start.max(retention_cutoff);
    if live_start <= query_end {
        let live_start_at = format_naive(live_start.with_timezone(&Shanghai).naive_local());
        rows.extend(
            load_window_actual_usage_rows_from_pool(
                &state.pool,
                &account_ids,
                &live_start_at,
                &query_end_at,
                None,
            )
            .await?,
        );
    }

    if query_start < retention_cutoff {
        let archive_end = query_end.min(retention_cutoff - ChronoDuration::seconds(1));
        if query_start <= archive_end {
            let archive_end_at = format_naive(archive_end.with_timezone(&Shanghai).naive_local());
            rows.extend(
                load_window_actual_usage_rows_from_archives(
                    &state.pool,
                    &account_ids,
                    &query_start_at,
                    &archive_end_at,
                    &state.config.archive_dir,
                )
                .await?,
            );
        }
    }

    let usage = fold_account_window_usage_rows(rows, &plans);
    apply_window_actual_usage_to_summaries(items, &usage);
    Ok(())
}
