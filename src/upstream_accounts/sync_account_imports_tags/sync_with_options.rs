use super::*;

pub(crate) async fn sync_upstream_account_by_id(
    state: &AppState,
    id: i64,
    cause: SyncCause,
) -> Result<Option<UpstreamAccountDetail>> {
    sync_upstream_account_by_id_with_options(state, id, cause, false).await
}
pub(crate) async fn sync_upstream_account_by_id_with_options(
    state: &AppState,
    id: i64,
    cause: SyncCause,
    skip_usage_snapshot: bool,
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

    let group_metadata = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        UpstreamAccountGroupMetadata::default()
    } else {
        match resolve_pool_account_group_proxy_routing_readiness(state, row.group_name.as_deref())
            .await?
        {
            PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => group_metadata,
            PoolAccountGroupProxyRoutingReadiness::Blocked(message) => bail!(message),
        }
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
            sync_oauth_account_with_options(state, &row, cause, skip_usage_snapshot).await
        }
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX => {
            if group_metadata.node_shunt_enabled {
                resolve_account_forward_proxy_scope(state, &row, Some(group_metadata)).await?;
            }
            sync_api_key_account(&state.pool, &row, cause).await
        }
        _ => bail!("unsupported account kind: {}", row.kind),
    };
    sync_result?;
    refresh_pool_model_routing_runtime_cache(state).await?;

    let refreshed_row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found after sync"))?;
    let now = Utc::now();
    if !is_account_selectable_for_fresh_assignment(&row, false, now)
        && is_account_selectable_for_fresh_assignment(&refreshed_row, false, now)
    {
        publish_pool_routing_availability(state);
    }

    let detail = load_upstream_account_detail_with_actual_usage(state, id)
        .await?
        .ok_or_else(|| anyhow!("account not found after sync"))?;
    Ok(Some(detail))
}

pub(super) async fn sync_oauth_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    cause: SyncCause,
) -> Result<()> {
    sync_oauth_account_with_options(state, row, cause, false).await
}

pub(super) async fn sync_oauth_account_with_options(
    state: &AppState,
    row: &UpstreamAccountRow,
    cause: SyncCause,
    skip_usage_snapshot: bool,
) -> Result<()> {
    let sync_source = sync_cause_action_source(cause);
    let now = Utc::now();
    let deferred_status =
        if row.status.trim().is_empty() || row.status == UPSTREAM_ACCOUNT_STATUS_SYNCING {
            UPSTREAM_ACCOUNT_STATUS_ACTIVE
        } else {
            row.status.as_str()
        };
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
            record_classified_account_sync_failure(
                &state.pool,
                row,
                deferred_status,
                sync_source,
                &err.to_string(),
            )
            .await?;
            return Ok(());
        }
    };
    let refresh_scope = usage_scope.clone();
    set_account_status(&state.pool, row.id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None).await?;

    if refresh_due && let Some(refresh_token) = oauth_refresh_token(&credentials) {
        match refresh_oauth_tokens_for_required_scope(state, &refresh_scope, refresh_token).await {
            Ok(response) => {
                let token_expires_at = apply_oauth_token_response(&mut credentials, response);
                persist_oauth_credentials(
                    &state.pool,
                    row.id,
                    crypto_key,
                    &credentials,
                    &token_expires_at,
                )
                .await?;
            }
            Err(err)
                if err
                    .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                    .is_some() =>
            {
                let throttle = err
                    .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                    .expect("checked throttle error");
                record_oauth_maintenance_throttled(
                    state,
                    row.id,
                    sync_source,
                    deferred_status,
                    throttle,
                )
                .await?;
                return Ok(());
            }
            Err(err) if is_reauth_error(&err) => {
                let proxy_snapshot = maintenance_proxy_snapshot_from_error(&err);
                record_account_sync_failure_with_proxy_snapshot(
                    &state.pool,
                    row.id,
                    deferred_status,
                    sync_source,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    &err.to_string(),
                    UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
                    None,
                    PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                    None,
                    false,
                    proxy_snapshot.as_ref(),
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
                let proxy_snapshot = maintenance_proxy_snapshot_from_error(&err);
                record_account_sync_failure_with_proxy_snapshot(
                    &state.pool,
                    row.id,
                    deferred_status,
                    sync_source,
                    next_status,
                    &err.to_string(),
                    reason_code,
                    http_status,
                    failure_kind,
                    route_failure_kind,
                    disposition == UpstreamAccountFailureDisposition::HardUnavailable,
                    proxy_snapshot.as_ref(),
                )
                .await?;
                return Ok(());
            }
        }
    }

    let mut latest_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared during sync"))?;
    if skip_usage_snapshot {
        mark_account_sync_success_with_proxy_snapshot(
            &state.pool,
            row.id,
            sync_source,
            if should_clear_route_failure_state_after_sync_success(&latest_row) {
                SyncSuccessRouteState::ClearFailureState
            } else {
                SyncSuccessRouteState::PreserveFailureState
            },
            None,
        )
        .await?;
        return Ok(());
    }
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

    let (snapshot, usage_proxy_snapshot) = match usage_result {
        Ok((snapshot, proxy_snapshot)) => (snapshot, Some(proxy_snapshot)),
        Err(err)
            if err
                .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                .is_some() =>
        {
            let throttle = err
                .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                .expect("checked throttle error");
            record_oauth_maintenance_throttled(
                state,
                row.id,
                sync_source,
                deferred_status,
                throttle,
            )
            .await?;
            return Ok(());
        }
        Err(err)
            if (err.to_string().contains("401") || err.to_string().contains("403"))
                && oauth_refresh_token(&credentials).is_some() =>
        {
            let refresh_token = oauth_refresh_token(&credentials).expect("checked refresh token");
            match refresh_oauth_tokens_for_required_scope(state, &refresh_scope, refresh_token)
                .await
            {
                Ok(response) => {
                    let mut refreshed = credentials.clone();
                    let token_expires_at = apply_oauth_token_response(&mut refreshed, response);
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
                        Ok((snapshot, proxy_snapshot)) => (snapshot, Some(proxy_snapshot)),
                        Err(retry_err)
                            if retry_err
                                .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                                .is_some() =>
                        {
                            let throttle = retry_err
                                .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                                .expect("checked throttle error");
                            record_oauth_maintenance_throttled(
                                state,
                                row.id,
                                sync_source,
                                deferred_status,
                                throttle,
                            )
                            .await?;
                            return Ok(());
                        }
                        Err(retry_err) => {
                            let proxy_snapshot = maintenance_proxy_snapshot_from_error(&retry_err);
                            record_classified_account_sync_failure_with_proxy_snapshot(
                                &state.pool,
                                &latest_row,
                                deferred_status,
                                sync_source,
                                &retry_err.to_string(),
                                proxy_snapshot.as_ref(),
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                }
                Err(refresh_err) if is_reauth_error(&refresh_err) => {
                    let proxy_snapshot = maintenance_proxy_snapshot_from_error(&refresh_err);
                    record_account_sync_failure_with_proxy_snapshot(
                        &state.pool,
                        row.id,
                        deferred_status,
                        sync_source,
                        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                        &refresh_err.to_string(),
                        UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
                        None,
                        PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                        None,
                        false,
                        proxy_snapshot.as_ref(),
                    )
                    .await?;
                    return Ok(());
                }
                Err(refresh_err)
                    if refresh_err
                        .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                        .is_some() =>
                {
                    let throttle = refresh_err
                        .downcast_ref::<AccountMaintenanceEgressThrottleError>()
                        .expect("checked throttle error");
                    record_oauth_maintenance_throttled(
                        state,
                        row.id,
                        sync_source,
                        deferred_status,
                        throttle,
                    )
                    .await?;
                    return Ok(());
                }
                Err(refresh_err) => {
                    let proxy_snapshot = maintenance_proxy_snapshot_from_error(&refresh_err);
                    record_classified_account_sync_failure_with_proxy_snapshot(
                        &state.pool,
                        &latest_row,
                        deferred_status,
                        sync_source,
                        &refresh_err.to_string(),
                        proxy_snapshot.as_ref(),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
        Err(err) => {
            let proxy_snapshot = maintenance_proxy_snapshot_from_error(&err);
            record_classified_account_sync_failure_with_proxy_snapshot(
                &state.pool,
                &latest_row,
                deferred_status,
                sync_source,
                &err.to_string(),
                proxy_snapshot.as_ref(),
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
            deferred_status,
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
            deferred_status,
            sync_source,
            UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
            "latest usage snapshot already shows an exhausted upstream usage limit window",
            PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED,
        )
        .await?;
        return Ok(());
    }
    mark_account_sync_success_with_proxy_snapshot(
        &state.pool,
        row.id,
        sync_source,
        if should_clear_route_failure_state_after_sync_success(&latest_row) {
            SyncSuccessRouteState::ClearFailureState
        } else {
            SyncSuccessRouteState::PreserveFailureState
        },
        usage_proxy_snapshot.as_ref(),
    )
    .await?;
    Ok(())
}
