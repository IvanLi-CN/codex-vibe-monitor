use super::*;

pub(crate) fn pool_route_binding_penalty_key(
    upstream_route_key: &str,
    proxy_binding_key: &str,
) -> String {
    format!("{upstream_route_key}\n{proxy_binding_key}")
}

pub(crate) fn pool_route_binding_failure_is_penalized(
    status: &str,
    failure_kind: Option<&str>,
) -> bool {
    status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
        && matches!(
            failure_kind,
            Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
                | Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
                | Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        )
}

pub(crate) async fn load_recent_route_binding_failure_penalties(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<String, i64>> {
    #[derive(Debug, FromRow)]
    struct RouteBindingAttemptRow {
        upstream_route_key: String,
        proxy_binding_key_snapshot: String,
        status: String,
        failure_kind: Option<String>,
    }

    let rows = sqlx::query_as::<_, RouteBindingAttemptRow>(
        r#"
        SELECT
            upstream_route_key,
            proxy_binding_key_snapshot,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        WHERE upstream_route_key IS NOT NULL
          AND proxy_binding_key_snapshot IS NOT NULL
          AND occurred_at >= datetime('now', ?1)
        ORDER BY occurred_at ASC, id ASC
        "#,
    )
    .bind(format!(
        "-{} seconds",
        POOL_ROUTE_BINDING_FAILURE_PENALTY_WINDOW_SECS
    ))
    .fetch_all(pool)
    .await?;

    let mut penalties = HashMap::new();
    for row in rows {
        let key = pool_route_binding_penalty_key(
            row.upstream_route_key.as_str(),
            row.proxy_binding_key_snapshot.as_str(),
        );
        if row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
            penalties.remove(&key);
            continue;
        }
        if pool_route_binding_failure_is_penalized(&row.status, row.failure_kind.as_deref()) {
            *penalties.entry(key).or_insert(0) += 1;
        }
    }
    Ok(penalties)
}

pub(crate) async fn route_binding_keys_for_candidate_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Vec<String> {
    let manager = state.forward_proxy.lock().await;
    match scope {
        ForwardProxyRouteScope::Automatic => Vec::new(),
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => manager
            .canonicalize_bound_proxy_key(proxy_key, None)
            .into_iter()
            .collect(),
        ForwardProxyRouteScope::BoundGroup {
            group_name,
            bound_proxy_keys,
        } => manager
            .current_bound_group_binding_key(group_name, bound_proxy_keys)
            .map(|key| vec![key])
            .unwrap_or_else(|| manager.selectable_bound_proxy_keys_in_order(bound_proxy_keys)),
        ForwardProxyRouteScope::BoundProxyKeys {
            scope_key,
            bound_proxy_keys,
        } => manager
            .current_bound_scope_binding_key(scope_key, bound_proxy_keys)
            .map(|key| vec![key])
            .unwrap_or_else(|| manager.selectable_bound_proxy_keys_in_order(bound_proxy_keys)),
    }
}

pub(crate) async fn route_binding_failure_penalty_for_account(
    state: &AppState,
    account: &PoolResolvedAccount,
    penalties: &HashMap<String, i64>,
) -> i64 {
    let upstream_route_key = account.upstream_route_key();
    route_binding_keys_for_candidate_scope(state, &account.forward_proxy_scope)
        .await
        .into_iter()
        .filter_map(|proxy_binding_key| {
            penalties
                .get(&pool_route_binding_penalty_key(
                    upstream_route_key.as_str(),
                    proxy_binding_key.as_str(),
                ))
                .copied()
        })
        .max()
        .unwrap_or(0)
}

pub(crate) async fn build_assigned_blocked_account(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: UpstreamAccountGroupMetadata,
    routing_source: PoolRoutingSelectionSource,
    message: String,
) -> Result<Option<PoolAssignedBlockedAccount>> {
    Ok(prepare_pool_account_identity_only(
        state,
        row,
        effective_rule,
        group_metadata,
        routing_source,
    )
    .await?
    .map(|account| PoolAssignedBlockedAccount {
        account,
        message,
        failure_kind: PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED,
    }))
}

pub(crate) async fn evaluate_live_pool_candidate(
    state: &AppState,
    row: &UpstreamAccountRow,
    candidate: &AccountRoutingCandidateRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: &UpstreamAccountGroupMetadata,
    node_shunt_assignments: &mut UpstreamAccountNodeShuntAssignments,
    routing_source: PoolRoutingSelectionSource,
    conversation_override: Option<&ConversationRoutingOverride>,
    now: DateTime<Utc>,
) -> Result<LivePoolCandidateEvaluation> {
    let group_metadata = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        UpstreamAccountGroupMetadata::default()
    } else {
        group_metadata.clone()
    };
    let conversation_proxy_scope = conversation_forward_proxy_scope(conversation_override);
    let build_evaluation =
        |eligibility, dispatch_state, resolved_account, assigned_blocked, blocked_message| {
            let runtime_last_selected_at = state
                .pool_account_selection_runtime
                .latest_selected_at(candidate.id, candidate.last_selected_at.as_deref());
            LivePoolCandidateEvaluation {
                score: build_pool_routing_candidate_score(
                    candidate,
                    effective_rule,
                    eligibility,
                    dispatch_state,
                    group_metadata.single_account_rotation_enabled,
                    runtime_last_selected_at,
                    now,
                ),
                resolved_account,
                assigned_blocked,
                blocked_message,
            }
        };

    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        let transit_proxy_scope = conversation_proxy_scope
            .clone()
            .unwrap_or_else(|| transit_account_forward_proxy_scope(row));
        let resolved_account = prepare_pool_account_with_scopes(
            state,
            row,
            effective_rule,
            group_metadata.clone(),
            transit_proxy_scope.clone(),
            transit_proxy_scope,
            routing_source,
        )
        .await?;
        return Ok(build_evaluation(
            if resolved_account.is_some() {
                PoolRoutingCandidateEligibility::Assignable
            } else {
                PoolRoutingCandidateEligibility::HardBlocked
            },
            if resolved_account.is_some() {
                PoolRoutingCandidateDispatchState::ReadyOnOwnedNode
            } else {
                PoolRoutingCandidateDispatchState::HardBlocked
            },
            resolved_account,
            None,
            None,
        ));
    }

    if group_metadata.node_shunt_enabled {
        if let Some(conversation_proxy_scope) = conversation_proxy_scope {
            let resolved_account = prepare_pool_account_with_scopes(
                state,
                row,
                effective_rule,
                group_metadata.clone(),
                ForwardProxyRouteScope::Automatic,
                conversation_proxy_scope,
                routing_source,
            )
            .await?;
            return Ok(build_evaluation(
                if resolved_account.is_some() {
                    PoolRoutingCandidateEligibility::Assignable
                } else {
                    PoolRoutingCandidateEligibility::HardBlocked
                },
                if resolved_account.is_some() {
                    PoolRoutingCandidateDispatchState::ReadyOnOwnedNode
                } else {
                    PoolRoutingCandidateDispatchState::HardBlocked
                },
                resolved_account,
                None,
                None,
            ));
        }

        let Some(group_name) = row
            .group_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            let message = missing_account_group_error_message();
            let assigned_blocked = build_assigned_blocked_account(
                state,
                row,
                effective_rule,
                group_metadata.clone(),
                routing_source,
                message.clone(),
            )
            .await?;
            return Ok(build_evaluation(
                PoolRoutingCandidateEligibility::HardBlocked,
                PoolRoutingCandidateDispatchState::HardBlocked,
                None,
                assigned_blocked,
                Some(message),
            ));
        };

        let slot_proxy_keys =
            canonical_group_bound_proxy_keys(state, &group_metadata.bound_proxy_keys).await;
        if slot_proxy_keys.is_empty() {
            let message = missing_group_bound_proxy_error_message(group_name);
            let assigned_blocked = build_assigned_blocked_account(
                state,
                row,
                effective_rule,
                group_metadata.clone(),
                routing_source,
                message.clone(),
            )
            .await?;
            return Ok(build_evaluation(
                PoolRoutingCandidateEligibility::HardBlocked,
                PoolRoutingCandidateDispatchState::HardBlocked,
                None,
                assigned_blocked,
                Some(message),
            ));
        }

        let refresh_proxy_scope =
            required_account_forward_proxy_scope(Some(group_name), slot_proxy_keys.clone())?;
        let selectable_proxy_keys =
            selectable_group_bound_proxy_keys(state, &slot_proxy_keys).await;

        if let Some(proxy_key) = node_shunt_assignments.account_proxy_keys.get(&row.id) {
            let dispatch_state = if selectable_proxy_keys.contains(proxy_key) {
                PoolRoutingCandidateDispatchState::ReadyOnOwnedNode
            } else {
                PoolRoutingCandidateDispatchState::RetryOriginalNode
            };
            let eligibility =
                if dispatch_state == PoolRoutingCandidateDispatchState::ReadyOnOwnedNode {
                    PoolRoutingCandidateEligibility::Assignable
                } else {
                    PoolRoutingCandidateEligibility::SoftDegraded
                };
            let resolved_account = prepare_pool_account_with_scopes(
                state,
                row,
                effective_rule,
                group_metadata.clone(),
                refresh_proxy_scope,
                conversation_proxy_scope
                    .clone()
                    .unwrap_or_else(|| ForwardProxyRouteScope::pinned(proxy_key.clone())),
                routing_source,
            )
            .await?;
            if resolved_account.is_none() {
                *node_shunt_assignments =
                    build_upstream_account_node_shunt_assignments(state).await?;
            }
            return Ok(build_evaluation(
                if resolved_account.is_some() {
                    eligibility
                } else {
                    PoolRoutingCandidateEligibility::HardBlocked
                },
                if resolved_account.is_some() {
                    dispatch_state
                } else {
                    PoolRoutingCandidateDispatchState::HardBlocked
                },
                resolved_account,
                None,
                None,
            ));
        }

        if !selectable_proxy_keys.is_empty() {
            let unoccupied_selectable_proxy_key = selectable_proxy_keys.iter().find(|proxy_key| {
                !node_shunt_assignments
                    .group_assigned_proxy_keys
                    .get(group_name)
                    .is_some_and(|assigned| assigned.contains(proxy_key.as_str()))
            });
            let dispatch_proxy_scope = if let Some(proxy_key) = unoccupied_selectable_proxy_key {
                ForwardProxyRouteScope::pinned(proxy_key.clone())
            } else {
                required_account_forward_proxy_scope(Some(group_name), selectable_proxy_keys)?
            };
            let resolved_account = prepare_pool_account_with_scopes(
                state,
                row,
                effective_rule,
                group_metadata.clone(),
                refresh_proxy_scope,
                conversation_proxy_scope
                    .clone()
                    .unwrap_or(dispatch_proxy_scope),
                routing_source,
            )
            .await?;
            if resolved_account.is_none() {
                *node_shunt_assignments =
                    build_upstream_account_node_shunt_assignments(state).await?;
            }
            return Ok(build_evaluation(
                if resolved_account.is_some() {
                    PoolRoutingCandidateEligibility::SoftDegraded
                } else {
                    PoolRoutingCandidateEligibility::HardBlocked
                },
                if resolved_account.is_some() {
                    PoolRoutingCandidateDispatchState::ReadyAfterMigration
                } else {
                    PoolRoutingCandidateDispatchState::HardBlocked
                },
                resolved_account,
                None,
                None,
            ));
        }

        let message = missing_selectable_group_bound_proxy_error_message(group_name);
        let assigned_blocked = build_assigned_blocked_account(
            state,
            row,
            effective_rule,
            group_metadata.clone(),
            routing_source,
            message.clone(),
        )
        .await?;
        return Ok(build_evaluation(
            PoolRoutingCandidateEligibility::HardBlocked,
            PoolRoutingCandidateDispatchState::HardBlocked,
            None,
            assigned_blocked,
            Some(message),
        ));
    }

    let refresh_proxy_scope = required_account_forward_proxy_scope(
        row.group_name.as_deref(),
        group_metadata.bound_proxy_keys.clone(),
    )?;
    let resolved_account = prepare_pool_account_with_scopes(
        state,
        row,
        effective_rule,
        group_metadata.clone(),
        refresh_proxy_scope.clone(),
        conversation_proxy_scope.unwrap_or(refresh_proxy_scope),
        routing_source,
    )
    .await?;
    Ok(build_evaluation(
        if resolved_account.is_some() {
            PoolRoutingCandidateEligibility::Assignable
        } else {
            PoolRoutingCandidateEligibility::HardBlocked
        },
        if resolved_account.is_some() {
            PoolRoutingCandidateDispatchState::ReadyOnOwnedNode
        } else {
            PoolRoutingCandidateDispatchState::HardBlocked
        },
        resolved_account,
        None,
        None,
    ))
}
