pub(crate) fn requested_model_matches_constraint(
    requested_model: &str,
    candidate_model: &str,
) -> bool {
    let requested_model = requested_model.trim();
    let candidate_model = candidate_model.trim();
    if requested_model.is_empty() || candidate_model.is_empty() {
        return false;
    }
    if requested_model.eq_ignore_ascii_case(candidate_model) {
        return true;
    }
    let requested_alias = dated_model_alias_base(requested_model).unwrap_or(requested_model);
    let candidate_alias = dated_model_alias_base(candidate_model).unwrap_or(candidate_model);
    requested_alias.eq_ignore_ascii_case(candidate_alias)
}

pub(crate) fn account_accepts_requested_model(
    requested_model: Option<&str>,
    rule: &EffectiveRoutingRule,
) -> bool {
    let Some(requested_model) = requested_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    if rule.available_models_defined
        && !rule
            .available_models
            .iter()
            .any(|candidate| requested_model_matches_constraint(requested_model, candidate))
    {
        return false;
    }
    !rule
        .system_denied_models
        .iter()
        .any(|candidate| requested_model_matches_constraint(requested_model, candidate))
}

pub(crate) fn apply_conversation_routing_override(
    rule: &mut EffectiveRoutingRule,
    override_policy: Option<&ConversationRoutingOverride>,
) {
    let Some(override_policy) = override_policy else {
        return;
    };
    if let Some(fast_mode_rewrite_mode) = override_policy.fast_mode_rewrite_mode {
        rule.fast_mode_rewrite_mode = fast_mode_rewrite_mode;
        rule.field_sources.fast_mode_rewrite_mode = "conversation".to_string();
    }
    if let Some(image_tool_rewrite_mode) = override_policy.image_tool_rewrite_mode {
        rule.image_tool_rewrite_mode = image_tool_rewrite_mode;
        rule.field_sources.image_tool_rewrite_mode = "conversation".to_string();
    }
    if let Some(available_models) = override_policy.available_models.as_ref() {
        rule.available_models = available_models.clone();
        rule.available_models_defined = true;
        rule.field_sources.available_models = "conversation".to_string();
    }
}

pub(crate) fn account_is_image_compatible(
    rewrite_mode: ImageToolRewriteMode,
    capability: ImageToolCapability,
) -> bool {
    match rewrite_mode {
        ImageToolRewriteMode::ForceAdd | ImageToolRewriteMode::FillMissing => true,
        ImageToolRewriteMode::ForceRemove => false,
        ImageToolRewriteMode::KeepOriginal => {
            !matches!(capability, ImageToolCapability::Unsupported)
        }
    }
}

pub(crate) fn account_accepts_requested_image_intent(
    image_intent: ImageIntent,
    rewrite_mode: ImageToolRewriteMode,
    capability: ImageToolCapability,
) -> bool {
    match image_intent {
        ImageIntent::Yes => account_is_image_compatible(rewrite_mode, capability),
        ImageIntent::DirectImage => !matches!(capability, ImageToolCapability::Unsupported),
        ImageIntent::No | ImageIntent::Unknown => true,
    }
}

pub(crate) async fn load_account_group_name_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, Option<String>>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, group_name FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(")")
        .build_query_as::<(i64, Option<String>)>()
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

pub(crate) async fn load_effective_routing_rules_for_accounts(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, EffectiveRoutingRule>> {
    let account_group_map = load_account_group_name_map(pool, account_ids).await?;
    if account_group_map.is_empty() {
        return Ok(HashMap::new());
    }

    let tags_by_account = load_account_tag_map(pool, account_ids).await?;
    let group_names = account_group_map
        .values()
        .filter_map(|group_name| normalize_optional_text(group_name.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let group_policy_overrides = load_group_routing_policy_override_map(pool, &group_names).await?;
    let account_policy_overrides = load_account_routing_policy_override_map(pool, account_ids).await?;
    let mut rules = HashMap::with_capacity(account_group_map.len());
    for (account_id, group_name) in account_group_map {
        let mut rule = build_effective_routing_rule(&[]);
        let normalized_group_name = normalize_optional_text(group_name.clone());
        if let Some(group_name) = normalized_group_name.as_ref()
            && let Some(group_policy) = group_policy_overrides.get(group_name)
        {
            apply_group_routing_policy_override(&mut rule, group_policy);
        }
        if let Some(tags) = tags_by_account.get(&account_id)
            && !tags.is_empty()
        {
            let tag_rule = build_effective_routing_rule(tags);
            apply_tag_layer_routing_policy(&mut rule, &tag_rule);
        }
        if let Some(account_policy) = account_policy_overrides.get(&account_id) {
            apply_account_routing_policy_override(&mut rule, account_policy);
        }
        rules.insert(account_id, rule);
    }
    Ok(rules)
}

pub(crate) fn routing_priority_rank(rule: Option<&EffectiveRoutingRule>) -> u8 {
    rule.map(|rule| rule.priority_tier)
        .unwrap_or_default()
        .routing_rank()
}

pub(crate) async fn load_effective_routing_rule_for_account(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<EffectiveRoutingRule> {
    Ok(
        load_effective_routing_rules_for_accounts(pool, &[account_id])
            .await?
            .remove(&account_id)
            .unwrap_or_else(|| build_effective_routing_rule(&[])),
    )
}

pub(crate) async fn load_effective_routing_rule_for_group(
    pool: &Pool<Sqlite>,
    group_name: Option<&str>,
) -> Result<EffectiveRoutingRule> {
    let mut rule = build_effective_routing_rule(&[]);
    let Some(group_name) = group_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
    else {
        return Ok(rule);
    };
    let mut group_policy_overrides =
        load_group_routing_policy_override_map(pool, &[group_name.clone()]).await?;
    if let Some(group_policy) = group_policy_overrides.remove(&group_name) {
        apply_group_routing_policy_override(&mut rule, &group_policy);
    }
    Ok(rule)
}

pub(crate) fn account_accepts_concurrency_limit(
    effective_load: i64,
    routing_source: PoolRoutingSelectionSource,
    rule: &EffectiveRoutingRule,
) -> bool {
    routing_source == PoolRoutingSelectionSource::StickyReuse
        || rule.concurrency_limit == 0
        || effective_load < rule.concurrency_limit
}

pub(crate) async fn account_accepts_sticky_assignment(
    _pool: &Pool<Sqlite>,
    account_id: i64,
    sticky_key: Option<&str>,
    source_account_id: Option<i64>,
    rule: &EffectiveRoutingRule,
    bypass_transfer_policy: bool,
) -> Result<bool> {
    let is_transfer = source_account_id.is_some_and(|source_id| source_id != account_id);
    let is_new_assignment = source_account_id.is_none();
    if is_new_assignment && rule.block_new_conversations {
        return Ok(false);
    }
    let Some(_) = sticky_key else {
        return Ok(true);
    };
    if !is_transfer && !is_new_assignment {
        return Ok(true);
    }
    if is_transfer && !bypass_transfer_policy && !rule.allow_cut_in {
        return Ok(false);
    }
    Ok(true)
}

pub(crate) async fn resolve_pool_account_group_proxy_routing_readiness(
    state: &AppState,
    group_name: Option<&str>,
) -> Result<PoolAccountGroupProxyRoutingReadiness> {
    let normalized_group_name = group_name.map(str::trim).filter(|value| !value.is_empty());
    let group_metadata = load_group_metadata(&state.pool, normalized_group_name).await?;
    if group_metadata.node_shunt_enabled {
        if normalized_group_name.is_none() {
            return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
                missing_account_group_error_message(),
            ));
        }
        return Ok(PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata));
    }
    let Some(group_name) = normalized_group_name else {
        return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
            missing_account_group_error_message(),
        ));
    };
    let scope = match load_required_account_forward_proxy_scope_from_group_metadata(
        state,
        Some(group_name),
    )
    .await
    {
        Ok(scope) => scope,
        Err(err) => {
            return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
                err.to_string(),
            ));
        }
    };
    let ForwardProxyRouteScope::BoundGroup {
        group_name,
        bound_proxy_keys,
    } = &scope
    else {
        unreachable!("strict pool account routing should never fall back to automatic");
    };
    let has_selectable_bound_proxy_keys = {
        let manager = state.forward_proxy.lock().await;
        manager.has_selectable_bound_proxy_keys(bound_proxy_keys)
    };
    if !has_selectable_bound_proxy_keys {
        return Ok(PoolAccountGroupProxyRoutingReadiness::Blocked(
            missing_selectable_group_bound_proxy_error_message(group_name),
        ));
    }
    Ok(PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata))
}

pub(crate) fn summarize_pool_group_proxy_blocked_messages(messages: &[String]) -> Option<String> {
    let mut seen = HashSet::new();
    let mut unique_messages = Vec::new();
    for message in messages {
        let normalized = message.trim();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.to_string()) {
            unique_messages.push(normalized.to_string());
        }
    }
    let first_message = unique_messages.first()?.clone();
    if unique_messages.len() == 1 {
        return Some(first_message);
    }
    Some(format!(
        "{first_message}; plus {} additional upstream account group routing configuration issue(s)",
        unique_messages.len() - 1
    ))
}

async fn canonical_group_bound_proxy_keys(
    state: &AppState,
    bound_proxy_keys: &[String],
) -> Vec<String> {
    let manager = state.forward_proxy.lock().await;
    let mut seen = HashSet::new();
    bound_proxy_keys
        .iter()
        .filter_map(|value| manager.canonicalize_bound_proxy_key(value, None))
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

async fn selectable_group_bound_proxy_keys(
    state: &AppState,
    bound_proxy_keys: &[String],
) -> Vec<String> {
    let manager = state.forward_proxy.lock().await;
    manager.selectable_bound_proxy_keys_in_order(bound_proxy_keys)
}

fn build_pool_resolved_account(
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: &UpstreamAccountGroupMetadata,
    auth: PoolResolvedAuth,
    upstream_base_url: Url,
    forward_proxy_scope: ForwardProxyRouteScope,
    routing_source: PoolRoutingSelectionSource,
) -> PoolResolvedAccount {
    PoolResolvedAccount {
        account_id: row.id,
        display_name: row.display_name.clone(),
        kind: row.kind.clone(),
        auth,
        group_name: row.group_name.clone(),
        bound_proxy_keys: group_metadata.bound_proxy_keys.clone(),
        forward_proxy_scope,
        single_account_rotation_enabled: group_metadata.single_account_rotation_enabled,
        upstream_429_retry_enabled: effective_rule.upstream_429_retry_enabled,
        upstream_429_max_retries: effective_rule.upstream_429_max_retries,
        fast_mode_rewrite_mode: effective_rule.fast_mode_rewrite_mode,
        image_tool_rewrite_mode: effective_rule.image_tool_rewrite_mode,
        image_tool_capability: decode_image_tool_capability(row.image_tool_capability.as_deref()),
        upstream_base_url,
        routing_source,
    }
}

pub(crate) fn conversation_forward_proxy_scope(
    override_policy: Option<&ConversationRoutingOverride>,
) -> Option<ForwardProxyRouteScope> {
    override_policy.and_then(|policy| {
        if !policy.forward_proxy_keys.is_empty() {
            Some(ForwardProxyRouteScope::bound_scope(
                policy.forward_proxy_scope_key.clone(),
                policy.forward_proxy_keys.clone(),
            ))
        } else {
            policy
                .forward_proxy_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|proxy_key| ForwardProxyRouteScope::pinned(proxy_key.to_string()))
        }
    })
}

async fn prepare_pool_account_with_scopes(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: UpstreamAccountGroupMetadata,
    refresh_proxy_scope: ForwardProxyRouteScope,
    forward_proxy_scope: ForwardProxyRouteScope,
    routing_source: PoolRoutingSelectionSource,
) -> Result<Option<PoolResolvedAccount>> {
    let Some(crypto_key) = state.upstream_accounts.crypto_key.as_ref() else {
        return Ok(None);
    };
    let Some(encrypted_credentials) = row.encrypted_credentials.as_deref() else {
        return Ok(None);
    };
    let upstream_base_url =
        resolve_pool_account_upstream_base_url(row, &state.config.openai_upstream_base_url)?;
    let credentials = decrypt_credentials(crypto_key, encrypted_credentials)?;
    match credentials {
        StoredCredentials::ApiKey(value) => Ok(Some(build_pool_resolved_account(
            row,
            effective_rule,
            &group_metadata,
            PoolResolvedAuth::ApiKey {
                authorization: format!("Bearer {}", value.api_key),
            },
            upstream_base_url,
            forward_proxy_scope,
            routing_source,
        ))),
        StoredCredentials::Oauth(mut value) => {
            let expires_at = row.token_expires_at.as_deref().and_then(parse_rfc3339_utc);
            let refresh_due = expires_at
                .map(|expires| {
                    expires
                        <= Utc::now()
                            + ChronoDuration::seconds(
                                state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                            )
                })
                .unwrap_or(true);
            let deferred_status = if row.status.trim().is_empty()
                || row.status == UPSTREAM_ACCOUNT_STATUS_SYNCING
            {
                UPSTREAM_ACCOUNT_STATUS_ACTIVE
            } else {
                row.status.as_str()
            };
            if refresh_due && let Some(refresh_token) = oauth_refresh_token(&value) {
                match refresh_oauth_tokens_for_required_scope(
                    state,
                    &refresh_proxy_scope,
                    refresh_token,
                )
                .await
                {
                    Ok(response) => {
                        let token_expires_at = apply_oauth_token_response(&mut value, response);
                        persist_oauth_credentials(
                            &state.pool,
                            row.id,
                            crypto_key,
                            &value,
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
                        let reason_message = format!(
                            "maintenance egress via {} is throttled for another {} seconds",
                            throttle.proxy_display_name, throttle.retry_after_secs
                        );
                        let now_iso = format_utc_iso(Utc::now());
                        record_account_maintenance_deferred(
                            &state.pool,
                            row.id,
                            UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
                            &reason_message,
                            &now_iso,
                            Some(&throttle.proxy_key),
                            Some(&throttle.proxy_display_name),
                            throttle.proxy_egress_ip.as_deref(),
                        )
                        .await?;
                        set_account_status(&state.pool, row.id, deferred_status, None).await?;
                        return Ok(None);
                    }
                    Err(err) if is_reauth_error(&err) => {
                        let err_text = err.to_string();
                        let now_iso = format_utc_iso(Utc::now());
                        let proxy_snapshot = maintenance_proxy_snapshot_from_error(&err);
                        sqlx::query(
                            r#"
                            UPDATE pool_upstream_accounts
                            SET status = ?2,
                                last_error = ?3,
                                last_error_at = ?4,
                                last_route_failure_at = ?4,
                                last_route_failure_kind = ?5,
                                cooldown_until = NULL,
                                consecutive_route_failures = consecutive_route_failures + 1,
                                temporary_route_failure_streak_started_at = NULL,
                                updated_at = ?4
                            WHERE id = ?1
                            "#,
                        )
                        .bind(row.id)
                        .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
                        .bind(&err_text)
                        .bind(&now_iso)
                        .bind(PROXY_FAILURE_UPSTREAM_HTTP_AUTH)
                        .execute(&state.pool)
                        .await?;
                        record_upstream_account_action_with_proxy_snapshot(
                            &state.pool,
                            row.id,
                            UpstreamAccountActionPayload {
                                action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                                source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                                reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED),
                                reason_message: Some(&err_text),
                                http_status: None,
                                failure_kind: Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
                                invoke_id: None,
                                sticky_key: None,
                                occurred_at: &now_iso,
                            },
                            proxy_snapshot.as_ref(),
                        )
                        .await?;
                        return Ok(None);
                    }
                    Err(err) => {
                        let err_text = err.to_string();
                        let proxy_snapshot = maintenance_proxy_snapshot_from_error(&err);
                        let (disposition, reason_code, next_status, http_status, failure_kind) =
                            classify_sync_failure(&row.kind, &err_text);
                        match disposition {
                            UpstreamAccountFailureDisposition::HardUnavailable => {
                                let now_iso = format_utc_iso(Utc::now());
                                sqlx::query(
                                    r#"
                                    UPDATE pool_upstream_accounts
                                    SET status = ?2,
                                        last_error = ?3,
                                        last_error_at = ?4,
                                        last_route_failure_at = ?4,
                                        last_route_failure_kind = ?5,
                                        cooldown_until = NULL,
                                        consecutive_route_failures = consecutive_route_failures + 1,
                                        temporary_route_failure_streak_started_at = NULL,
                                        updated_at = ?4
                                    WHERE id = ?1
                                    "#,
                                )
                                .bind(row.id)
                                .bind(next_status.unwrap_or(UPSTREAM_ACCOUNT_STATUS_ERROR))
                                .bind(&err_text)
                                .bind(&now_iso)
                                .bind(failure_kind)
                                .execute(&state.pool)
                                .await?;
                        record_upstream_account_action_with_proxy_snapshot(
                            &state.pool,
                            row.id,
                            UpstreamAccountActionPayload {
                                action: UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE,
                                source: UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL,
                                        reason_code: Some(reason_code),
                                        reason_message: Some(&err_text),
                                        http_status,
                                        failure_kind: Some(failure_kind),
                                        invoke_id: None,
                                        sticky_key: None,
                                        occurred_at: &now_iso,
                                    },
                                    proxy_snapshot.as_ref(),
                                )
                                .await?;
                            }
                            UpstreamAccountFailureDisposition::RateLimited
                            | UpstreamAccountFailureDisposition::Retryable => {
                                apply_pool_route_cooldown_failure(
                                    &state.pool,
                                    row.id,
                                    UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                                    None,
                                    &err_text,
                                    failure_kind,
                                    reason_code,
                                    http_status.unwrap_or(StatusCode::BAD_GATEWAY),
                                    5,
                                    None,
                                )
                                .await?;
                            }
                        }
                        return Ok(None);
                    }
                }
            }

            Ok(Some(build_pool_resolved_account(
                row,
                effective_rule,
                &group_metadata,
                PoolResolvedAuth::Oauth {
                    access_token: value.access_token,
                    chatgpt_account_id: row.chatgpt_account_id.clone(),
                },
                upstream_base_url,
                forward_proxy_scope,
                routing_source,
            )))
        }
    }
}

pub(crate) async fn prepare_pool_account_identity_only(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: UpstreamAccountGroupMetadata,
    routing_source: PoolRoutingSelectionSource,
) -> Result<Option<PoolResolvedAccount>> {
    let Some(crypto_key) = state.upstream_accounts.crypto_key.as_ref() else {
        return Ok(None);
    };
    let Some(encrypted_credentials) = row.encrypted_credentials.as_deref() else {
        return Ok(None);
    };
    let upstream_base_url =
        resolve_pool_account_upstream_base_url(row, &state.config.openai_upstream_base_url)?;
    let credentials = decrypt_credentials(crypto_key, encrypted_credentials)?;
    let auth = match credentials {
        StoredCredentials::ApiKey(value) => PoolResolvedAuth::ApiKey {
            authorization: format!("Bearer {}", value.api_key),
        },
        StoredCredentials::Oauth(value) => PoolResolvedAuth::Oauth {
            access_token: value.access_token,
            chatgpt_account_id: row.chatgpt_account_id.clone(),
        },
    };
    Ok(Some(build_pool_resolved_account(
        row,
        effective_rule,
        &group_metadata,
        auth,
        upstream_base_url,
        ForwardProxyRouteScope::Automatic,
        routing_source,
    )))
}

pub(crate) async fn prepare_pool_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: UpstreamAccountGroupMetadata,
    node_shunt_assignments: &UpstreamAccountNodeShuntAssignments,
    conversation_override: Option<&ConversationRoutingOverride>,
) -> Result<Option<PoolResolvedAccount>> {
    let conversation_proxy_scope = conversation_forward_proxy_scope(conversation_override);
    let account_proxy_scope = account_bound_forward_proxy_scope(row);
    let refresh_proxy_scope = conversation_proxy_scope.clone().unwrap_or(
        account_proxy_scope.clone().unwrap_or(required_account_forward_proxy_scope(
            row.group_name.as_deref(),
            group_metadata.bound_proxy_keys.clone(),
        )?),
    );
    let forward_proxy_scope = conversation_proxy_scope.unwrap_or(
        account_proxy_scope.unwrap_or(resolve_account_forward_proxy_scope_from_assignments(
            row.id,
            row.group_name.as_deref(),
            &group_metadata,
            node_shunt_assignments,
        )?),
    );
    prepare_pool_account_with_scopes(
        state,
        row,
        effective_rule,
        group_metadata,
        refresh_proxy_scope,
        forward_proxy_scope,
        PoolRoutingSelectionSource::FreshAssignment,
    )
    .await
}

pub(crate) fn is_account_selectable_for_sticky_reuse(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    _now: DateTime<Utc>,
) -> bool {
    row.provider == UPSTREAM_ACCOUNT_PROVIDER_CODEX
        && row.enabled != 0
        && row.encrypted_credentials.is_some()
        && (row.status == UPSTREAM_ACCOUNT_STATUS_ACTIVE
            || is_account_rate_limited_for_routing(row, snapshot_exhausted))
}

pub(crate) fn is_account_selectable_for_fresh_assignment(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    is_routing_eligible_account(row)
        && !snapshot_exhausted
        && !is_account_rate_limited_for_routing(row, snapshot_exhausted)
        && !is_account_degraded_for_routing(row, snapshot_exhausted, now)
}

pub(crate) fn is_account_degraded_for_routing(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    is_routing_eligible_account(row)
        && !snapshot_exhausted
        && upstream_account_degraded_state_is_current(
            &row.status,
            row.cooldown_until.as_deref(),
            row.last_error_at.as_deref(),
            row.last_route_failure_at.as_deref(),
            row.last_route_failure_kind.as_deref(),
            row.last_action_reason_code.as_deref(),
            row.temporary_route_failure_streak_started_at.as_deref(),
            now,
        )
}

pub(crate) fn is_pool_account_routing_candidate(row: &UpstreamAccountRow) -> bool {
    row.provider == UPSTREAM_ACCOUNT_PROVIDER_CODEX
        && row.enabled != 0
        && row.status == UPSTREAM_ACCOUNT_STATUS_ACTIVE
}

pub(crate) fn is_routing_eligible_account(row: &UpstreamAccountRow) -> bool {
    is_pool_account_routing_candidate(row) && row.encrypted_credentials.is_some()
}

pub(crate) fn is_account_rate_limited_for_routing(row: &UpstreamAccountRow, snapshot_exhausted: bool) -> bool {
    if row.provider != UPSTREAM_ACCOUNT_PROVIDER_CODEX
        || row.enabled == 0
        || row.encrypted_credentials.is_none()
    {
        return false;
    }
    let quota_exhausted_hard_stop =
        route_failure_kind_is_quota_exhausted(row.last_route_failure_kind.as_deref());
    snapshot_exhausted
        || quota_exhausted_hard_stop
        || account_reason_is_rate_limited(row.last_action_reason_code.as_deref())
}

pub(crate) async fn load_account_routing_candidates(
    pool: &Pool<Sqlite>,
    excluded_ids: &HashSet<i64>,
) -> Result<Vec<AccountRoutingCandidateRow>> {
    let active_sticky_cutoff = format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES),
    );
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.plan_type
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS plan_type,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_window_minutes,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_window_minutes,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            account.local_primary_limit,
            account.local_secondary_limit,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance,
            account.last_selected_at,
            (
                SELECT COUNT(*)
                FROM pool_sticky_routes route
                WHERE route.account_id = account.id
                  AND route.last_seen_at >=
        "#,
    );
    query.push_bind(&active_sticky_cutoff).push(
        r#"
            ) AS active_sticky_conversations
        FROM pool_upstream_accounts account
        WHERE account.provider =
        "#,
    );
    query
        .push_bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .push(" AND account.enabled = 1");
    if !excluded_ids.is_empty() {
        query.push(" AND account.id NOT IN (");
        {
            let mut separated = query.separated(", ");
            for account_id in excluded_ids {
                separated.push_bind(account_id);
            }
        }
        query.push(")");
    }
    query.push(" ORDER BY account.id ASC");

    query
        .build_query_as::<AccountRoutingCandidateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_account_routing_candidate(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<AccountRoutingCandidateRow>> {
    sqlx::query_as::<_, AccountRoutingCandidateRow>(
        r#"
        SELECT
            account.id,
            (
                SELECT sample.plan_type
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS plan_type,
            (
                SELECT sample.secondary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_used_percent,
            (
                SELECT sample.secondary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_window_minutes,
            (
                SELECT sample.secondary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS secondary_resets_at,
            (
                SELECT sample.primary_used_percent
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_used_percent,
            (
                SELECT sample.primary_window_minutes
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_window_minutes,
            (
                SELECT sample.primary_resets_at
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS primary_resets_at,
            account.local_primary_limit,
            account.local_secondary_limit,
            (
                SELECT sample.credits_has_credits
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_has_credits,
            (
                SELECT sample.credits_unlimited
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_unlimited,
            (
                SELECT sample.credits_balance
                FROM pool_upstream_account_limit_samples sample
                WHERE sample.account_id = account.id
                ORDER BY sample.captured_at DESC
                LIMIT 1
            ) AS credits_balance,
            account.last_selected_at,
            (
                SELECT COUNT(*)
                FROM pool_sticky_routes route
                WHERE route.account_id = account.id
                  AND route.last_seen_at >= ?2
            ) AS active_sticky_conversations
        FROM pool_upstream_accounts account
        WHERE account.id = ?1
        "#,
    )
    .bind(account_id)
    .bind(format_utc_iso(
        Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES),
    ))
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_transport_decode_sticky_escape_account_ids(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashSet<i64>> {
    if account_ids.is_empty() {
        return Ok(HashSet::new());
    }

    #[derive(Debug, FromRow)]
    struct EscapeRow {
        upstream_account_id: i64,
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        WITH ranked AS (
            SELECT
                upstream_account_id,
                failure_kind,
                ROW_NUMBER() OVER (
                    PARTITION BY upstream_account_id
                    ORDER BY occurred_at DESC, id DESC
                ) AS attempt_rank
            FROM pool_upstream_request_attempts
            WHERE upstream_account_id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(
        r#"
            )
              AND route_mode =
        "#,
    );
    query.push_bind(INVOCATION_ROUTE_MODE_POOL).push(
        r#"
              AND endpoint = '/v1/responses'
              AND phase IN (
        "#,
    );
    query
        .push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
        .push(", ")
        .push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
        .push(
            r#"
              )
        )
        SELECT upstream_account_id
        FROM ranked
        WHERE attempt_rank <= 2
        GROUP BY upstream_account_id
        HAVING COUNT(*) = 2
           AND SUM(
                CASE
                    WHEN failure_kind =
        "#,
        );
    query
        .push_bind(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        .push(
            r#"
                    THEN 1
                    ELSE 0
                END
            ) = 2
        "#,
        );

    let rows = query.build_query_as::<EscapeRow>().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| row.upstream_account_id)
        .collect())
}

pub(crate) fn compare_routing_candidates(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
) -> std::cmp::Ordering {
    compare_routing_candidates_at(lhs, rhs, Utc::now())
}

pub(crate) fn compare_routing_candidates_at(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
    now: DateTime<Utc>,
) -> std::cmp::Ordering {
    let lhs_capacity = lhs.capacity_profile();
    let rhs_capacity = rhs.capacity_profile();
    let lhs_over_soft_limit = lhs.effective_load() > lhs_capacity.soft_limit;
    let rhs_over_soft_limit = rhs.effective_load() > rhs_capacity.soft_limit;
    let lhs_score = lhs.scarcity_score(now);
    let rhs_score = rhs.scarcity_score(now);
    lhs_over_soft_limit
        .cmp(&rhs_over_soft_limit)
        .then_with(|| {
            lhs_score
                .partial_cmp(&rhs_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| lhs.effective_load().cmp(&rhs.effective_load()))
        .then_with(|| lhs.last_selected_at.cmp(&rhs.last_selected_at))
        .then_with(|| lhs.id.cmp(&rhs.id))
}
