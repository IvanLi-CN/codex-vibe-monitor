use super::*;
pub(crate) async fn sync_upstream_account_by_id(
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

pub(crate) async fn sync_api_key_account(
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
            &row.status,
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

pub(crate) fn refresh_http_status(error: &str) -> Option<StatusCode> {
    if error.contains("400") {
        Some(StatusCode::BAD_REQUEST)
    } else if error.contains("401") {
        Some(StatusCode::UNAUTHORIZED)
    } else if error.contains("402") {
        Some(StatusCode::PAYMENT_REQUIRED)
    } else if error.contains("403") {
        Some(StatusCode::FORBIDDEN)
    } else if error.contains("429") {
        Some(StatusCode::TOO_MANY_REQUESTS)
    } else if error.contains("500") {
        Some(StatusCode::INTERNAL_SERVER_ERROR)
    } else if error.contains("502") {
        Some(StatusCode::BAD_GATEWAY)
    } else if error.contains("503") {
        Some(StatusCode::SERVICE_UNAVAILABLE)
    } else if error.contains("504") {
        Some(StatusCode::GATEWAY_TIMEOUT)
    } else {
        None
    }
}

#[derive(Debug)]
pub(crate) enum OAuthUsageRefreshOutcome {
    Succeeded {
        snapshot: NormalizedUsageSnapshot,
        proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
        lease_token: String,
    },
    Partial {
        snapshot: NormalizedUsageSnapshot,
        proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
        lease_token: String,
    },
    Failed {
        error: String,
        http_status: Option<StatusCode>,
        proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
        lease_token: String,
    },
    Deferred {
        proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
        lease_token: Option<String>,
    },
}

impl OAuthUsageRefreshOutcome {
    fn lease_token(&self) -> Option<&str> {
        match self {
            Self::Succeeded { lease_token, .. }
            | Self::Partial { lease_token, .. }
            | Self::Failed { lease_token, .. } => Some(lease_token),
            Self::Deferred { lease_token, .. } => lease_token.as_deref(),
        }
    }
}

struct OAuthUsageRefreshLeaseCleanupGuard {
    pool: Pool<Sqlite>,
    account_id: i64,
    operation: &'static str,
    lease_token: String,
    effective_plan_type: Option<String>,
    retention_days: u64,
    armed: bool,
}

impl OAuthUsageRefreshLeaseCleanupGuard {
    fn new(
        pool: &Pool<Sqlite>,
        account_id: i64,
        operation: &'static str,
        lease_token: &str,
        effective_plan_type: Option<&str>,
        retention_days: u64,
    ) -> Self {
        Self {
            pool: pool.clone(),
            account_id,
            operation,
            lease_token: lease_token.to_string(),
            effective_plan_type: effective_plan_type.map(str::to_string),
            retention_days,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for OAuthUsageRefreshLeaseCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let pool = self.pool.clone();
        let account_id = self.account_id;
        let operation = self.operation;
        let lease_token = self.lease_token.clone();
        let effective_plan_type = self.effective_plan_type.clone();
        let retention_days = self.retention_days;
        drop(handle.spawn(async move {
            if let Err(err) = persist_unknown_reserve_usage_snapshot_with_lease(
                &pool,
                account_id,
                effective_plan_type.as_deref(),
                retention_days,
                Some(&lease_token),
            )
            .await
            {
                warn!(
                    account_id,
                    operation,
                    error = %err,
                    "failed to invalidate Reserve usage snapshot after cancellation"
                );
            }
            if let Err(err) =
                release_upstream_account_refresh_lease(&pool, account_id, operation, &lease_token)
                    .await
            {
                warn!(
                    account_id,
                    operation,
                    error = %err,
                    "upstream account refresh lease cleanup failed after cancellation"
                );
            }
        }));
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_usage_refresh_failure(
    state: &AppState,
    row: &UpstreamAccountRow,
    lease_token: &str,
    source: &str,
    raw_error: &str,
    error_code: &str,
    error_message: &str,
    partial: bool,
    http_status: Option<StatusCode>,
    proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
) -> Result<OAuthUsageRefreshOutcome> {
    persist_unknown_reserve_usage_snapshot_with_lease(
        &state.pool,
        row.id,
        row.plan_type.as_deref(),
        state.config.upstream_accounts_history_retention_days,
        Some(lease_token),
    )
    .await?;
    let applied = record_upstream_account_refresh_failure_with_lease(
        &state.pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
        source,
        raw_error,
        error_code,
        error_message,
        partial,
        http_status,
        Some(lease_token),
    )
    .await?;
    if !applied {
        return Ok(OAuthUsageRefreshOutcome::Deferred {
            proxy_snapshot,
            lease_token: None,
        });
    }
    if http_status
        .is_some_and(|status| matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN))
    {
        schedule_upstream_account_refresh_retry(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
            "access_token_rejected",
            "额度接口拒绝当前 Access Token，下一轮独立凭据刷新将重试。",
            row.encrypted_credentials.as_deref(),
        )
        .await?;
    }
    Ok(OAuthUsageRefreshOutcome::Failed {
        error: usage_refresh_failure_message(error_code, error_message),
        http_status,
        proxy_snapshot,
        lease_token: lease_token.to_string(),
    })
}

pub(crate) async fn refresh_oauth_credentials_if_due(
    state: &AppState,
    row: &UpstreamAccountRow,
    cause: SyncCause,
    refresh_scope: &ForwardProxyRouteScope,
) -> Result<()> {
    refresh_oauth_credentials_if_due_with_source(
        state,
        row,
        sync_cause_action_source(cause),
        refresh_scope,
    )
    .await
}

pub(crate) async fn refresh_oauth_credentials_if_due_with_source(
    state: &AppState,
    row: &UpstreamAccountRow,
    source: &str,
    refresh_scope: &ForwardProxyRouteScope,
) -> Result<()> {
    refresh_oauth_credentials_with_source(state, row, source, refresh_scope, false).await
}

pub(crate) async fn refresh_oauth_credentials_now_with_source(
    state: &AppState,
    row: &UpstreamAccountRow,
    source: &str,
    refresh_scope: &ForwardProxyRouteScope,
) -> Result<()> {
    refresh_oauth_credentials_with_source(state, row, source, refresh_scope, true).await
}

async fn refresh_oauth_credentials_with_source(
    state: &AppState,
    row: &UpstreamAccountRow,
    source: &str,
    refresh_scope: &ForwardProxyRouteScope,
    force: bool,
) -> Result<()> {
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(row.id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    let current_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    let row = &current_row;
    let now = Utc::now();
    let credential_status = load_upstream_account_refresh_status(
        &state.pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await?;
    if !force && credential_refresh_attempt_is_blocked(&credential_status, now) {
        return Ok(());
    }
    let token_expiry = row
        .encrypted_credentials
        .as_deref()
        .and_then(|encrypted| {
            state
                .upstream_accounts
                .crypto_key
                .as_ref()
                .and_then(|key| decrypt_credentials(key, encrypted).ok())
        })
        .and_then(|credentials| match credentials {
            StoredCredentials::Oauth(credentials) => {
                parse_access_token_expiry(&credentials.access_token)
            }
            StoredCredentials::ApiKey(_) => None,
        })
        .or_else(|| row.token_expires_at.as_deref().and_then(parse_rfc3339_utc));
    let access_token_rejected =
        credential_status.error_code.as_deref() == Some("access_token_rejected");
    let refresh_due = force
        || access_token_rejected
        || if let Some(expires) = token_expiry {
            expires
                <= now
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        } else {
            credential_status
                .last_success_at
                .as_deref()
                .and_then(parse_rfc3339_utc)
                .map(|last_success| last_success + ChronoDuration::days(8) <= now)
                .unwrap_or(true)
        };
    if !refresh_due {
        return Ok(());
    }

    let Some(lease_token) = begin_upstream_account_refresh_attempt_with_lease(
        &state.pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        source,
    )
    .await?
    else {
        return Ok(());
    };

    let refresh_result = async {
        let crypto_key = match state.upstream_accounts.crypto_key.as_ref() {
        Some(key) => key,
        None => {
            let (code, message) = classify_refresh_error(
                "account writes are disabled",
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            );
            record_upstream_account_refresh_failure_with_lease(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                source,
                "account writes are disabled",
                &code,
                &message,
                false,
                None,
                Some(&lease_token),
            )
            .await?;
            return Ok(());
        }
        };
        let encrypted = match row.encrypted_credentials.as_deref() {
        Some(value) => value,
        None => {
            let (code, message) = classify_refresh_error(
                "missing encrypted OAuth credentials",
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            );
            record_upstream_account_refresh_failure_with_lease(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                source,
                "missing encrypted OAuth credentials",
                &code,
                &message,
                false,
                None,
                Some(&lease_token),
            )
            .await?;
            return Ok(());
        }
        };
        let decrypted = match decrypt_credentials(crypto_key, encrypted) {
        Ok(credentials) => credentials,
        Err(err) => {
            let (code, message) = classify_refresh_error(
                &err.to_string(),
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            );
            record_upstream_account_refresh_failure_with_lease(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                source,
                &err.to_string(),
                &code,
                &message,
                false,
                None,
                Some(&lease_token),
            )
            .await?;
            return Ok(());
        }
        };
        let StoredCredentials::Oauth(mut credentials) = decrypted else {
        let (code, message) = classify_refresh_error(
            "unexpected credential kind for OAuth account",
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        );
        record_upstream_account_refresh_failure_with_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            source,
            "unexpected credential kind for OAuth account",
            &code,
            &message,
            false,
            None,
            Some(&lease_token),
        )
        .await?;
        return Ok(());
        };
        let Some(refresh_token) = oauth_refresh_token(&credentials) else {
        record_upstream_account_refresh_failure_with_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            source,
            "refresh token missing",
            "refresh_token_missing",
            "Refresh Token 缺失，需要重新授权。",
            false,
            None,
            Some(&lease_token),
        )
        .await?;
        return Ok(());
        };

        match refresh_oauth_tokens_for_required_scope(state, refresh_scope, refresh_token).await {
        Ok(response) => {
            let response_token_metadata = oauth_token_response_lifecycle_metadata(
                &response,
                &response.access_token,
                Utc::now(),
            );
            let response_token_expires_at = apply_oauth_token_response(&mut credentials, response);
            let access_token_expiry = parse_access_token_expiry(&credentials.access_token);
            if credentials.access_token.matches('.').count() == 2 && access_token_expiry.is_none() {
                warn!(
                    account_id = row.id,
                    operation = UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                    stage = "access_token_jwt",
                    error_code = "access_token_jwt_parse_failed",
                    "refreshed access token JWT exp could not be parsed; using the OAuth expires_in fallback"
                );
            }
            let stored_token_expires_at = parse_access_token_expiry(&credentials.access_token)
                .map(format_utc_iso)
                .unwrap_or_else(|| response_token_expires_at.clone());
            let next_attempt_at = access_token_expiry
                .or_else(|| parse_rfc3339_utc(&response_token_expires_at))
                .map(|expires| {
                    format_utc_iso(
                        expires
                            - ChronoDuration::seconds(
                                state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                            ),
                    )
                })
                .or_else(|| Some(format_utc_iso(Utc::now() + ChronoDuration::days(8))));
            match persist_oauth_credentials_and_complete_refresh_with_lease(
                &state.pool,
                row.id,
                crypto_key,
                &credentials,
                &stored_token_expires_at,
                Some(&response_token_metadata),
                source,
                next_attempt_at.as_deref(),
                Some(&lease_token),
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(err) => {
                    let (code, message) = classify_refresh_error(
                        &err.to_string(),
                        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                    );
                    record_upstream_account_refresh_failure_with_lease(
                        &state.pool,
                        row.id,
                        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                        source,
                        &err.to_string(),
                        &code,
                        &message,
                        false,
                        None,
                        Some(&lease_token),
                    )
                    .await?;
                }
            }
        }
        Err(err) => {
            if let Some(throttle) = err.downcast_ref::<AccountMaintenanceEgressThrottleError>() {
                let reason_message = format!(
                    "maintenance egress via {} is throttled for another {} seconds",
                    throttle.proxy_display_name, throttle.retry_after_secs
                );
                if !defer_upstream_account_refresh_attempt_with_lease(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                    source,
                    throttle.retry_after_secs,
                    &reason_message,
                    Some(&lease_token),
                )
                .await?
                {
                    return Ok(());
                }
                record_oauth_maintenance_throttled(
                    state,
                    row.id,
                    source,
                    if row.status == UPSTREAM_ACCOUNT_STATUS_SYNCING {
                        UPSTREAM_ACCOUNT_STATUS_ACTIVE
                    } else {
                        row.status.as_str()
                    },
                    throttle,
                )
                .await?;
                return Ok(());
            }
            let (code, message) = classify_refresh_error(
                &err.to_string(),
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            );
            record_upstream_account_refresh_failure_with_lease(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                source,
                &err.to_string(),
                &code,
                &message,
                false,
                refresh_http_status(&err.to_string()),
                Some(&lease_token),
            )
            .await?;
        }
        }
        Ok(())
    }
    .await;
    if refresh_result.is_err()
        && let Err(err) = release_upstream_account_refresh_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            &lease_token,
        )
        .await
    {
        warn!(
            account_id = row.id,
            operation = UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            source,
            error = %err,
            "upstream account credential refresh lease cleanup failed after an internal error"
        );
    }
    refresh_result
}

pub(crate) async fn refresh_oauth_usage_windows(
    state: &AppState,
    _row: &UpstreamAccountRow,
    source: &str,
) -> Result<OAuthUsageRefreshOutcome> {
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(_row.id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    let current_row = load_upstream_account_row(&state.pool, _row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared before usage refresh"))?;
    let row = &current_row;
    let operation = UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS;
    let Some(lease_token) =
        begin_upstream_account_refresh_attempt_with_lease(&state.pool, row.id, operation, source)
            .await?
    else {
        return Ok(OAuthUsageRefreshOutcome::Deferred {
            proxy_snapshot: None,
            lease_token: None,
        });
    };
    let mut lease_cleanup_guard = OAuthUsageRefreshLeaseCleanupGuard::new(
        &state.pool,
        row.id,
        operation,
        &lease_token,
        row.plan_type.as_deref(),
        state.config.upstream_accounts_history_retention_days,
    );
    let refresh_result = async {
        let crypto_key = match state.upstream_accounts.crypto_key.as_ref() {
            Some(key) => key,
            None => {
                return record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    "account writes are disabled",
                    "credentials_unavailable",
                    "账号凭据不可用，等待下一分钟重试。",
                    false,
                    None,
                    None,
                )
                .await;
            }
        };
        let encrypted = match row.encrypted_credentials.as_deref() {
            Some(value) => value,
            None => {
                return record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    "missing encrypted OAuth credentials",
                    "credentials_missing",
                    "OAuth 凭据缺失，等待下一分钟重试。",
                    false,
                    None,
                    None,
                )
                .await;
            }
        };
        let decrypted = match decrypt_credentials(crypto_key, encrypted) {
            Ok(credentials) => credentials,
            Err(err) => {
                let (code, message) = classify_refresh_error(&err.to_string(), operation);
                return record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    &err.to_string(),
                    &code,
                    &message,
                    false,
                    None,
                    None,
                )
                .await;
            }
        };
        let StoredCredentials::Oauth(credentials) = decrypted else {
            return record_usage_refresh_failure(
                state,
                row,
                &lease_token,
                source,
                "unexpected credential kind for OAuth account",
                "credentials_kind_invalid",
                "OAuth 凭据类型无效，等待下一分钟重试。",
                false,
                None,
                None,
            )
            .await;
        };
        let usage_scope = match resolve_account_forward_proxy_scope_for_sync(state, row, None).await
        {
            Ok(scope) => scope,
            Err(err) if is_group_node_shunt_unassigned_message(&err.to_string()) => {
                let (code, message) = classify_refresh_error(&err.to_string(), operation);
                record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    &err.to_string(),
                    &code,
                    &message,
                    false,
                    refresh_http_status(&err.to_string()),
                    None,
                )
                .await?;
                return Err(err);
            }
            Err(err) => {
                let (code, message) = classify_refresh_error(&err.to_string(), operation);
                return record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    &err.to_string(),
                    &code,
                    &message,
                    false,
                    refresh_http_status(&err.to_string()),
                    None,
                )
                .await;
            }
        };
        let usage_result = fetch_usage_snapshot_via_forward_proxy(
            state,
            &usage_scope,
            &state.config,
            &credentials.access_token,
            row.chatgpt_account_id.as_deref(),
        )
        .await;
        let (snapshot, usage_proxy_snapshot) = match usage_result {
            Ok(value) => value,
            Err(err) => {
                if let Some(throttle) = err.downcast_ref::<AccountMaintenanceEgressThrottleError>()
                {
                    let reason_message = format!(
                        "maintenance egress via {} is throttled for another {} seconds",
                        throttle.proxy_display_name, throttle.retry_after_secs
                    );
                    if !defer_upstream_account_refresh_attempt_with_lease(
                        &state.pool,
                        row.id,
                        operation,
                        source,
                        throttle.retry_after_secs,
                        &reason_message,
                        Some(&lease_token),
                    )
                    .await?
                    {
                        return Ok(OAuthUsageRefreshOutcome::Deferred {
                            proxy_snapshot: Some(AccountMaintenanceProxySnapshot {
                                proxy_key: throttle.proxy_key.clone(),
                                proxy_display_name: throttle.proxy_display_name.clone(),
                                proxy_egress_ip: throttle.proxy_egress_ip.clone(),
                            }),
                            lease_token: None,
                        });
                    }
                    record_oauth_maintenance_throttled(
                        state,
                        row.id,
                        source,
                        if row.status == UPSTREAM_ACCOUNT_STATUS_SYNCING {
                            UPSTREAM_ACCOUNT_STATUS_ACTIVE
                        } else {
                            row.status.as_str()
                        },
                        throttle,
                    )
                    .await?;
                    return Ok(OAuthUsageRefreshOutcome::Deferred {
                        proxy_snapshot: Some(AccountMaintenanceProxySnapshot {
                            proxy_key: throttle.proxy_key.clone(),
                            proxy_display_name: throttle.proxy_display_name.clone(),
                            proxy_egress_ip: throttle.proxy_egress_ip.clone(),
                        }),
                        lease_token: Some(lease_token.clone()),
                    });
                }
                let (code, message) = classify_refresh_error(&err.to_string(), operation);
                return record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    &err.to_string(),
                    &code,
                    &message,
                    false,
                    refresh_http_status(&err.to_string()),
                    maintenance_proxy_snapshot_from_error(&err),
                )
                .await;
            }
        };
        let has_parse_evidence =
            snapshot.observed_window_candidates > 0 || snapshot.window_parse_failure_count > 0;
        let usable_window_count = if has_parse_evidence {
            snapshot.parsed_window_count
        } else {
            u8::from(snapshot.primary.is_some()) + u8::from(snapshot.secondary.is_some())
        };
        if usable_window_count == 0 {
            return record_usage_refresh_failure(
                state,
                row,
                &lease_token,
                source,
                "usage response did not contain a primary or secondary window",
                "usage_windows_missing",
                "额度窗口响应未包含主要或次要额度窗口，等待下一分钟重试。",
                false,
                None,
                Some(usage_proxy_snapshot),
            )
            .await;
        }
        let effective_snapshot_plan_type =
            match resolve_snapshot_plan_type(&state.pool, row, &snapshot).await {
                Ok(value) => value,
                Err(err) => {
                    let (code, message) = classify_refresh_error(&err.to_string(), operation);
                    return record_usage_refresh_failure(
                        state,
                        row,
                        &lease_token,
                        source,
                        &err.to_string(),
                        &code,
                        &message,
                        false,
                        None,
                        Some(usage_proxy_snapshot),
                    )
                    .await;
                }
            };
        let snapshot_persisted = match persist_usage_snapshot_with_lease(
            &state.pool,
            row.id,
            effective_snapshot_plan_type.as_deref(),
            &snapshot,
            state.config.upstream_accounts_history_retention_days,
            Some(&lease_token),
        )
        .await
        {
            Ok(true) => true,
            Ok(false) => {
                return Ok(OAuthUsageRefreshOutcome::Deferred {
                    proxy_snapshot: Some(usage_proxy_snapshot),
                    lease_token: None,
                });
            }
            Err(err) => {
                return record_usage_refresh_failure(
                    state,
                    row,
                    &lease_token,
                    source,
                    &err.to_string(),
                    "usage_snapshot_persist_failed",
                    "额度窗口数据保存失败，等待下一分钟重试。",
                    false,
                    None,
                    Some(usage_proxy_snapshot),
                )
                .await;
            }
        };
        debug_assert!(snapshot_persisted);
        let has_partial_window_data = has_parse_evidence && snapshot.window_parse_failure_count > 0;
        if !has_partial_window_data {
            if !complete_upstream_account_refresh_success_with_lease(
                &state.pool,
                row.id,
                operation,
                source,
                None,
                Some(&lease_token),
            )
            .await?
            {
                return Ok(OAuthUsageRefreshOutcome::Deferred {
                    proxy_snapshot: Some(usage_proxy_snapshot),
                    lease_token: None,
                });
            }
            Ok(OAuthUsageRefreshOutcome::Succeeded {
                snapshot,
                proxy_snapshot: Some(usage_proxy_snapshot),
                lease_token: lease_token.clone(),
            })
        } else {
            let applied = record_upstream_account_refresh_failure_with_lease(
                &state.pool,
                row.id,
                operation,
                source,
                "usage response contained partially parsed quota windows",
                "usage_window_partial",
                "额度窗口更新不完整，已保留可用数据并将在下一分钟重试。",
                true,
                None,
                Some(&lease_token),
            )
            .await?;
            if !applied {
                return Ok(OAuthUsageRefreshOutcome::Deferred {
                    proxy_snapshot: Some(usage_proxy_snapshot),
                    lease_token: None,
                });
            }
            Ok(OAuthUsageRefreshOutcome::Partial {
                snapshot,
                proxy_snapshot: Some(usage_proxy_snapshot),
                lease_token: lease_token.clone(),
            })
        }
    }
    .await;
    if refresh_result.is_err()
        && let Err(err) =
            release_upstream_account_refresh_lease(&state.pool, row.id, operation, &lease_token)
                .await
    {
        warn!(
            account_id = row.id,
            operation,
            source,
            error = %err,
            "upstream account refresh lease cleanup failed after an internal error"
        );
    }
    lease_cleanup_guard.disarm();
    refresh_result
}

fn credential_refresh_attempt_is_blocked(
    status: &UpstreamAccountRefreshStatus,
    now: DateTime<Utc>,
) -> bool {
    if status.state == UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH {
        return true;
    }
    if matches!(
        status.state.as_str(),
        UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING
            | UPSTREAM_ACCOUNT_REFRESH_STATE_PARTIAL
            | UPSTREAM_ACCOUNT_REFRESH_STATE_ERROR
    ) {
        return status
            .next_attempt_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .is_some_and(|next| next > now);
    }
    if status.state == UPSTREAM_ACCOUNT_REFRESH_STATE_REFRESHING {
        return status
            .last_attempt_at
            .as_deref()
            .and_then(parse_rfc3339_utc)
            .is_some_and(|attempt| {
                attempt + ChronoDuration::seconds(UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS)
                    > now
            });
    }
    false
}

async fn refresh_upstream_account_usage_by_id_with_result(
    state: &AppState,
    id: i64,
    load_detail: bool,
) -> Result<(Option<UpstreamAccountDetail>, bool)> {
    let row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        bail!("usage window refresh only supports OAuth accounts");
    }
    if row.enabled == 0 {
        bail!("disabled accounts cannot refresh usage windows");
    }
    let was_selectable = is_account_selectable_for_fresh_assignment(&row, false, Utc::now());
    let usage_outcome = refresh_oauth_usage_windows(
        state,
        &row,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
    )
    .await?;
    let reserve_usage_refresh_succeeded =
        matches!(&usage_outcome, OAuthUsageRefreshOutcome::Succeeded { .. });
    let mut usage_apply_cleanup_guard = usage_outcome.lease_token().map(|lease_token| {
        OAuthUsageRefreshLeaseCleanupGuard::new(
            &state.pool,
            id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            lease_token,
            row.plan_type.as_deref(),
            state.config.upstream_accounts_history_retention_days,
        )
    });
    let outcome_row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared after manual usage refresh"))?;
    let deferred_status = if outcome_row.status.trim().is_empty()
        || outcome_row.status == UPSTREAM_ACCOUNT_STATUS_SYNCING
    {
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    } else {
        outcome_row.status.as_str()
    };
    let apply_result =
        apply_manual_usage_refresh_outcome(state, &outcome_row, deferred_status, usage_outcome)
            .await;
    if apply_result.is_ok()
        && let Some(guard) = usage_apply_cleanup_guard.as_mut()
    {
        guard.disarm();
    }
    apply_result?;
    let refreshed_row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared after manual usage refresh"))?;
    if !was_selectable
        && is_account_selectable_for_fresh_assignment(&refreshed_row, false, Utc::now())
    {
        publish_pool_routing_availability(state);
    }
    let detail = if load_detail {
        load_upstream_account_detail_with_actual_usage(state, id).await?
    } else {
        None
    };
    Ok((detail, reserve_usage_refresh_succeeded))
}

pub(crate) async fn refresh_upstream_account_usage_by_id(
    state: &AppState,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    Ok(
        refresh_upstream_account_usage_by_id_with_result(state, id, true)
            .await?
            .0,
    )
}

pub(crate) async fn refresh_upstream_account_usage_for_reserve_by_id(
    state: &AppState,
    id: i64,
) -> Result<bool> {
    Ok(
        refresh_upstream_account_usage_by_id_with_result(state, id, false)
            .await?
            .1,
    )
}

async fn apply_manual_usage_refresh_outcome(
    state: &AppState,
    row: &UpstreamAccountRow,
    deferred_status: &str,
    usage_outcome: OAuthUsageRefreshOutcome,
) -> Result<()> {
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(row.id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    let current_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared before applying manual usage refresh"))?;
    let row = &current_row;
    let credential_reauth_required = load_upstream_account_refresh_status(
        &state.pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await?
    .state
        == UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH
        || row.status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH;
    let lease_token = usage_outcome.lease_token().map(str::to_owned);
    if let Some(lease_token) = lease_token.as_deref()
        && !touch_upstream_account_refresh_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            lease_token,
        )
        .await?
    {
        return Ok(());
    }
    let apply_result = async {
        match usage_outcome {
        OAuthUsageRefreshOutcome::Succeeded {
            snapshot: _,
            proxy_snapshot,
            ..
        } => {
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
            } else if persisted_usage_sample_is_exhausted(
                load_latest_usage_sample(&state.pool, row.id).await?.as_ref(),
            ) {
                let latest_row = load_upstream_account_row(&state.pool, row.id)
                    .await?
                    .ok_or_else(|| anyhow!("account disappeared after manual usage refresh"))?;
                if route_failure_kind_is_quota_exhausted(
                    latest_row.last_route_failure_kind.as_deref(),
                ) {
                    record_account_sync_recovery_blocked(
                        &state.pool,
                        row.id,
                        deferred_status,
                        UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                        UPSTREAM_ACCOUNT_STATUS_ERROR,
                        UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
                        "latest usage snapshot still shows an exhausted upstream usage limit window",
                        latest_row.last_error.as_deref().or(row.last_error.as_deref()),
                        latest_row.last_route_failure_kind.as_deref(),
                    )
                    .await?;
                } else {
                    record_account_sync_hard_unavailable(
                        &state.pool,
                        row.id,
                        deferred_status,
                        UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                        UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
                        "latest usage snapshot already shows an exhausted upstream usage limit window",
                        PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED,
                    )
                    .await?;
                }
            } else {
                mark_account_operation_success_with_proxy_snapshot(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                    SyncSuccessRouteState::ClearFailureState,
                    UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_SUCCEEDED,
                    "usage_windows_refresh_ok",
                    proxy_snapshot.as_ref(),
                )
                .await?;
            }
        }
        OAuthUsageRefreshOutcome::Partial {
            ..
        } => {
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
            } else if persisted_usage_sample_is_exhausted(
                load_latest_usage_sample(&state.pool, row.id).await?.as_ref(),
            ) {
                let latest_row = load_upstream_account_row(&state.pool, row.id)
                    .await?
                    .ok_or_else(|| anyhow!("account disappeared after partial usage refresh"))?;
                if route_failure_kind_is_quota_exhausted(
                    latest_row.last_route_failure_kind.as_deref(),
                ) {
                    record_account_sync_recovery_blocked(
                        &state.pool,
                        row.id,
                        deferred_status,
                        UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                        UPSTREAM_ACCOUNT_STATUS_ERROR,
                        UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
                        "latest usage snapshot still shows an exhausted upstream usage limit window",
                        latest_row.last_error.as_deref().or(row.last_error.as_deref()),
                        latest_row.last_route_failure_kind.as_deref(),
                    )
                    .await?;
                } else {
                    record_account_sync_hard_unavailable(
                        &state.pool,
                        row.id,
                        deferred_status,
                        UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                        UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
                        "latest usage snapshot already shows an exhausted upstream usage limit window",
                        PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED,
                    )
                    .await?;
                }
            } else {
                set_account_status(&state.pool, row.id, deferred_status, None).await?;
            }
        }
        OAuthUsageRefreshOutcome::Failed {
            error,
            http_status,
            proxy_snapshot,
            ..
        } => {
            let latest_row = load_upstream_account_row(&state.pool, row.id)
                .await?
                .ok_or_else(|| anyhow!("account disappeared after manual usage refresh failure"))?;
            let classified_error = http_status
                .filter(|status| !error.contains(&format!("HTTP {}", status.as_u16())))
                .map(|status| format!("上游返回 HTTP {}: {error}", status.as_u16()))
                .unwrap_or(error);
            if let Some(status @ (StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)) = http_status {
                record_account_sync_failure_with_proxy_snapshot(
                    &state.pool,
                    latest_row.id,
                    deferred_status,
                    UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                    deferred_status,
                    &classified_error,
                    UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
                    Some(status),
                    PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                    latest_row.last_route_failure_kind.as_deref(),
                    false,
                    proxy_snapshot.as_ref(),
                )
                .await?;
            } else {
                record_classified_account_sync_failure_with_proxy_snapshot(
                    &state.pool,
                    &latest_row,
                    deferred_status,
                    UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH,
                    &classified_error,
                    proxy_snapshot.as_ref(),
                )
                .await?;
            }
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
            }
        }
        OAuthUsageRefreshOutcome::Deferred { .. } => {
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
            }
        }
        }
        Ok(())
    }
    .await;
    if let Some(lease_token) = lease_token {
        release_upstream_account_refresh_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            &lease_token,
        )
        .await?;
    }
    apply_result
}

pub(crate) async fn refresh_upstream_account_credentials_by_id(
    state: &AppState,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        bail!("credential refresh only supports OAuth accounts");
    }
    if row.enabled == 0 {
        bail!("disabled accounts cannot refresh credentials");
    }
    let was_selectable = is_account_selectable_for_fresh_assignment(&row, false, Utc::now());
    let refresh_scope = resolve_account_forward_proxy_scope_for_sync(state, &row, None).await?;
    refresh_oauth_credentials_now_with_source(
        state,
        &row,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_CREDENTIAL_REFRESH,
        &refresh_scope,
    )
    .await?;
    let latest_row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared during manual credential refresh"))?;
    let credential_status = load_upstream_account_refresh_status(
        &state.pool,
        id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await?;
    if credential_status.state == UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH {
        set_account_status(
            &state.pool,
            id,
            UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
            Some("OAuth 凭据刷新需要重新授权。"),
        )
        .await?;
    } else if credential_status.state == UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED
        && latest_row.status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
    {
        set_account_status(&state.pool, id, UPSTREAM_ACCOUNT_STATUS_ACTIVE, None).await?;
    }
    let refreshed_row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| {
            anyhow!("account disappeared after manual credential refresh status update")
        })?;
    if !was_selectable
        && is_account_selectable_for_fresh_assignment(&refreshed_row, false, Utc::now())
    {
        publish_pool_routing_availability(state);
    }
    load_upstream_account_detail_with_actual_usage(state, id).await
}

pub(crate) async fn sync_oauth_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    cause: SyncCause,
) -> Result<()> {
    let sync_source = sync_cause_action_source(cause);
    let refresh_scope = match resolve_account_forward_proxy_scope_for_sync(state, row, None).await {
        Ok(scope) => Some(scope),
        Err(err) => {
            let (code, message) = classify_refresh_error(
                &err.to_string(),
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            );
            record_upstream_account_refresh_failure(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
                sync_source,
                &err.to_string(),
                &code,
                &message,
                false,
                refresh_http_status(&err.to_string()),
            )
            .await?;
            None
        }
    };
    if let Some(refresh_scope) = refresh_scope.as_ref() {
        if row.status != UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH {
            set_account_status(&state.pool, row.id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None).await?;
        }
        if cause != SyncCause::PostCreate {
            refresh_oauth_credentials_if_due(state, row, cause, refresh_scope).await?;
        }
    }
    let usage_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared during usage window refresh"))?;
    let usage_outcome = refresh_oauth_usage_windows(state, &usage_row, sync_source).await?;
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(row.id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    let outcome_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared after usage window refresh"))?;
    let deferred_status = if outcome_row.status.trim().is_empty()
        || outcome_row.status == UPSTREAM_ACCOUNT_STATUS_SYNCING
    {
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    } else {
        outcome_row.status.as_str()
    };
    let credential_reauth_required = load_upstream_account_refresh_status(
        &state.pool,
        row.id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await?
    .state
        == UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH
        || outcome_row.status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH;
    let lease_token = usage_outcome.lease_token().map(str::to_owned);
    if let Some(lease_token) = lease_token.as_deref()
        && !touch_upstream_account_refresh_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            lease_token,
        )
        .await?
    {
        return Ok(());
    }
    let apply_result = async {
        match usage_outcome {
        OAuthUsageRefreshOutcome::Succeeded {
            snapshot: _,
            proxy_snapshot,
            ..
        } => {
            let latest_row = load_upstream_account_row(&state.pool, row.id)
                .await?
                .ok_or_else(|| anyhow!("account disappeared after usage snapshot persisted"))?;
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
                return Ok(());
            }
            if route_failure_kind_is_quota_exhausted(latest_row.last_route_failure_kind.as_deref())
                && persisted_usage_sample_is_exhausted(
                    load_latest_usage_sample(&state.pool, row.id).await?.as_ref(),
                )
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
            if persisted_usage_sample_is_exhausted(
                load_latest_usage_sample(&state.pool, row.id).await?.as_ref(),
            ) {
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
                proxy_snapshot.as_ref(),
            )
            .await?;
        }
        OAuthUsageRefreshOutcome::Partial {
            ..
        } => {
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
                return Ok(());
            }
            let latest_row = load_upstream_account_row(&state.pool, row.id)
                .await?
                .ok_or_else(|| anyhow!("account disappeared after partial usage refresh"))?;
            if persisted_usage_sample_is_exhausted(
                load_latest_usage_sample(&state.pool, row.id).await?.as_ref(),
            ) {
                if route_failure_kind_is_quota_exhausted(
                    latest_row.last_route_failure_kind.as_deref(),
                ) {
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
                } else {
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
                }
            } else {
                set_account_status(&state.pool, row.id, deferred_status, None).await?;
            }
        }
        OAuthUsageRefreshOutcome::Failed {
            error,
            http_status,
            proxy_snapshot,
            ..
        } => {
            let latest_row = load_upstream_account_row(&state.pool, row.id)
                .await?
                .ok_or_else(|| anyhow!("account disappeared after usage refresh failure"))?;
            let classified_error = http_status
                .filter(|status| !error.contains(&format!("HTTP {}", status.as_u16())))
                .map(|status| format!("上游返回 HTTP {}: {error}", status.as_u16()))
                .unwrap_or(error);
            if let Some(status @ (StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)) = http_status {
                record_account_sync_failure_with_proxy_snapshot(
                    &state.pool,
                    latest_row.id,
                    deferred_status,
                    sync_source,
                    deferred_status,
                    &classified_error,
                    UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
                    Some(status),
                    PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
                    latest_row.last_route_failure_kind.as_deref(),
                    false,
                    proxy_snapshot.as_ref(),
                )
                .await?;
            } else {
                record_classified_account_sync_failure_with_proxy_snapshot(
                    &state.pool,
                    &latest_row,
                    deferred_status,
                    sync_source,
                    &classified_error,
                    proxy_snapshot.as_ref(),
                )
                .await?;
            }
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
            }
        }
        OAuthUsageRefreshOutcome::Deferred { .. } => {
            if credential_reauth_required {
                set_account_status(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    Some("OAuth 凭据刷新需要重新授权。"),
                )
                .await?;
            }
        }
        }
        Ok(())
    }
    .await;
    if let Some(lease_token) = lease_token {
        release_upstream_account_refresh_lease(
            &state.pool,
            row.id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
            &lease_token,
        )
        .await?;
    }
    apply_result
}

pub(crate) async fn record_oauth_maintenance_throttled(
    state: &AppState,
    account_id: i64,
    sync_source: &str,
    next_status: &str,
    throttle: &AccountMaintenanceEgressThrottleError,
) -> Result<()> {
    let reason_message = format!(
        "maintenance egress via {} is throttled for another {} seconds",
        throttle.proxy_display_name, throttle.retry_after_secs
    );
    record_account_maintenance_deferred(
        &state.pool,
        account_id,
        sync_source,
        &reason_message,
        &format_utc_iso(Utc::now()),
        Some(&throttle.proxy_key),
        Some(&throttle.proxy_display_name),
        throttle.proxy_egress_ip.as_deref(),
    )
    .await?;
    let effective_next_status = if load_upstream_account_refresh_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await?
    .state
        == UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH
    {
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
    } else {
        next_status
    };
    set_account_status(&state.pool, account_id, effective_next_status, None).await
}

pub(crate) async fn persist_oauth_credentials(
    pool: &Pool<Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    persist_oauth_credentials_with_executor(
        &mut tx,
        account_id,
        crypto_key,
        credentials,
        token_expires_at,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn persist_oauth_credentials_and_complete_refresh(
    pool: &Pool<Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
    source: &str,
    next_attempt_at: Option<&str>,
) -> Result<()> {
    let _ = persist_oauth_credentials_and_complete_refresh_with_lease(
        pool,
        account_id,
        crypto_key,
        credentials,
        token_expires_at,
        None,
        source,
        next_attempt_at,
        None,
    )
    .await?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Lease-fenced OAuth persistence keeps credential, expiry, source, retry, and fencing inputs explicit."
)]
pub(crate) async fn persist_oauth_credentials_and_complete_refresh_with_lease(
    pool: &Pool<Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
    lifecycle_metadata: Option<&OAuthTokenLifecycleMetadata>,
    source: &str,
    next_attempt_at: Option<&str>,
    expected_lease_token: Option<&str>,
) -> Result<bool> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if !refresh_lease_is_current(
        tx.as_mut(),
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        expected_lease_token,
    )
    .await?
    {
        return Ok(false);
    }
    persist_oauth_credentials_with_executor(
        &mut tx,
        account_id,
        crypto_key,
        credentials,
        token_expires_at,
        lifecycle_metadata,
    )
    .await?;
    if !complete_upstream_account_refresh_success_state_with_executor_and_lease(
        &mut tx,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        source,
        next_attempt_at,
        expected_lease_token,
    )
    .await?
    {
        return Ok(false);
    }
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = NULL,
            last_error_at = NULL,
            last_route_failure_at = NULL,
            last_route_failure_kind = NULL,
            cooldown_until = NULL,
            consecutive_route_failures = 0,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?1 AND status = ?4
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(format_utc_iso(Utc::now()))
    .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;

    if let Err(err) = record_upstream_account_refresh_success_event(
        pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        source,
    )
    .await
    {
        warn!(
            account_id,
            operation = UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            source,
            error = %err,
            "OAuth credentials persisted but success history event could not be recorded"
        );
    }
    Ok(true)
}

pub(crate) async fn persist_usage_snapshot(
    pool: &Pool<Sqlite>,
    account_id: i64,
    effective_plan_type: Option<&str>,
    snapshot: &NormalizedUsageSnapshot,
    retention_days: u64,
) -> Result<()> {
    let _ = persist_usage_snapshot_with_lease(
        pool,
        account_id,
        effective_plan_type,
        snapshot,
        retention_days,
        None,
    )
    .await?;
    Ok(())
}

pub(crate) async fn persist_usage_snapshot_with_lease(
    pool: &Pool<Sqlite>,
    account_id: i64,
    effective_plan_type: Option<&str>,
    snapshot: &NormalizedUsageSnapshot,
    retention_days: u64,
    expected_lease_token: Option<&str>,
) -> Result<bool> {
    persist_usage_snapshot_with_versions(
        pool,
        account_id,
        effective_plan_type,
        snapshot,
        retention_days,
        expected_lease_token,
        None,
    )
    .await
}

pub(crate) async fn apply_imported_oauth_probe_result(
    state: &AppState,
    account_id: i64,
    probe: &ImportedOauthProbeOutcome,
) -> Result<Option<String>> {
    let credential_refresh_lock = state.upstream_accounts.credential_refresh_lock(account_id);
    let _credential_refresh_guard = credential_refresh_lock.lock().await;
    apply_imported_oauth_probe_result_locked(state, account_id, probe).await
}
pub(crate) async fn apply_imported_oauth_probe_result_locked(
    state: &AppState,
    account_id: i64,
    probe: &ImportedOauthProbeOutcome,
) -> Result<Option<String>> {
    apply_imported_oauth_probe_result_locked_with_expected_usage(state, account_id, probe, None)
        .await
}

pub(crate) async fn apply_imported_oauth_probe_result_locked_with_expected_usage(
    state: &AppState,
    account_id: i64,
    probe: &ImportedOauthProbeOutcome,
    expected_usage_version: Option<Option<UsageSnapshotVersion>>,
) -> Result<Option<String>> {
    if let Some(snapshot) = probe.usage_snapshot.as_ref() {
        let before = load_upstream_account_row(&state.pool, account_id).await?;
        let now = Utc::now();
        let was_selectable = before
            .as_ref()
            .is_some_and(|row| is_account_selectable_for_fresh_assignment(row, false, now));
        let persisted = persist_usage_snapshot_with_versions(
            &state.pool,
            account_id,
            probe.claims.chatgpt_plan_type.as_deref(),
            snapshot,
            state.config.upstream_accounts_history_retention_days,
            None,
            expected_usage_version,
        )
        .await?;
        if !persisted {
            bail!(OAUTH_USAGE_SNAPSHOT_CONFLICT_MESSAGE);
        }
        mark_account_sync_success_with_proxy_snapshot(
            &state.pool,
            account_id,
            UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT,
            SyncSuccessRouteState::ClearFailureState,
            probe.maintenance_proxy_snapshot.as_ref(),
        )
        .await?;
        let became_selectable = load_upstream_account_row(&state.pool, account_id)
            .await?
            .is_some_and(|row| is_account_selectable_for_fresh_assignment(&row, false, Utc::now()));
        if !was_selectable && became_selectable {
            publish_pool_routing_availability(state);
        }
    }
    Ok(probe.usage_snapshot_warning.clone())
}

pub(crate) async fn publish_new_account_routing_availability_if_selectable(
    state: &AppState,
    account_id: i64,
) {
    match load_upstream_account_row(&state.pool, account_id).await {
        Ok(Some(row)) if is_account_selectable_for_fresh_assignment(&row, false, Utc::now()) => {
            publish_pool_routing_availability(state);
        }
        Ok(_) => {}
        Err(err) => {
            warn!(
                account_id,
                error = %err,
                "failed to inspect a newly created account for routing availability"
            );
        }
    }
}

pub(crate) async fn persist_imported_oauth_existing_inner(
    state: &AppState,
    account_id: i64,
    probe: ImportedOauthProbeOutcome,
) -> Result<Option<String>, (StatusCode, String)> {
    persist_imported_oauth_existing_inner_with_versions(state, account_id, probe, None, None).await
}
