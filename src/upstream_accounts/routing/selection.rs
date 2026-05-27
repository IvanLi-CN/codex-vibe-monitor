#[derive(Debug, Clone)]
struct LivePoolCandidateEvaluation {
    score: PoolRoutingCandidateScore,
    resolved_account: Option<PoolResolvedAccount>,
    assigned_blocked: Option<PoolAssignedBlockedAccount>,
    blocked_message: Option<String>,
}

const POOL_ROUTE_BINDING_FAILURE_PENALTY_WINDOW_SECS: i64 = 300;

pub(crate) fn compare_pool_routing_candidate_scores(
    lhs: &PoolRoutingCandidateScore,
    rhs: &PoolRoutingCandidateScore,
) -> std::cmp::Ordering {
    let lhs_requires_retry_original =
        lhs.dispatch_state == PoolRoutingCandidateDispatchState::RetryOriginalNode;
    let rhs_requires_retry_original =
        rhs.dispatch_state == PoolRoutingCandidateDispatchState::RetryOriginalNode;
    // Hard-blocked candidates are filtered before sort. Ready candidates should always beat
    // "retry original unavailable node" fallbacks. Among sendable candidates, soft-limit
    // pressure still demotes overflow accounts first, then recent route+proxy transport
    // failures demote a bad combination before account priority/scarcity tie-breakers.
    lhs_requires_retry_original
        .cmp(&rhs_requires_retry_original)
        .then_with(|| {
            lhs.capacity_lane
                .rank()
                .cmp(&rhs.capacity_lane.rank())
                .then_with(|| {
                    lhs.route_binding_failure_penalty
                        .cmp(&rhs.route_binding_failure_penalty)
                })
                .then_with(|| lhs.routing_priority_rank.cmp(&rhs.routing_priority_rank))
                .then_with(|| lhs.eligibility.rank().cmp(&rhs.eligibility.rank()))
                .then_with(|| lhs.dispatch_state.rank().cmp(&rhs.dispatch_state.rank()))
                .then_with(|| {
                    compare_reset_proximity_for_rotation_candidates(
                        lhs.single_account_rotation_enabled,
                        lhs.secondary_reset_proximity_secs,
                        rhs.single_account_rotation_enabled,
                        rhs.secondary_reset_proximity_secs,
                    )
                })
                .then_with(|| {
                    compare_reset_proximity_for_rotation_candidates(
                        lhs.single_account_rotation_enabled,
                        lhs.primary_reset_proximity_secs,
                        rhs.single_account_rotation_enabled,
                        rhs.primary_reset_proximity_secs,
                    )
                })
                .then_with(|| lhs.scarcity_score.total_cmp(&rhs.scarcity_score))
                .then_with(|| lhs.effective_load.cmp(&rhs.effective_load))
                .then_with(|| lhs.last_selected_at.cmp(&rhs.last_selected_at))
                .then_with(|| lhs.account_id.cmp(&rhs.account_id))
        })
}

fn compare_reset_proximity_for_rotation_candidates(
    lhs_enabled: bool,
    lhs_reset: Option<i64>,
    rhs_enabled: bool,
    rhs_reset: Option<i64>,
) -> std::cmp::Ordering {
    if lhs_enabled && rhs_enabled {
        compare_optional_reset_proximity(lhs_reset, rhs_reset)
    } else {
        std::cmp::Ordering::Equal
    }
}

fn compare_optional_reset_proximity(lhs: Option<i64>, rhs: Option<i64>) -> std::cmp::Ordering {
    match (lhs, rhs) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn reset_proximity_secs(resets_at: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    resets_at
        .and_then(parse_rfc3339_utc)
        .map(|reset| reset.signed_duration_since(now).num_seconds().abs())
}

fn build_pool_routing_candidate_score(
    candidate: &AccountRoutingCandidateRow,
    effective_rule: &EffectiveRoutingRule,
    eligibility: PoolRoutingCandidateEligibility,
    dispatch_state: PoolRoutingCandidateDispatchState,
    single_account_rotation_enabled: bool,
    now: DateTime<Utc>,
) -> PoolRoutingCandidateScore {
    let capacity_lane = if candidate.effective_load() <= candidate.capacity_profile().soft_limit {
        PoolRoutingCandidateCapacityLane::Primary
    } else {
        PoolRoutingCandidateCapacityLane::Overflow
    };
    PoolRoutingCandidateScore {
        eligibility,
        route_binding_failure_penalty: 0,
        routing_priority_rank: routing_priority_rank(Some(effective_rule)),
        capacity_lane,
        dispatch_state,
        single_account_rotation_enabled,
        secondary_reset_proximity_secs: reset_proximity_secs(
            candidate.secondary_resets_at.as_deref(),
            now,
        ),
        primary_reset_proximity_secs: reset_proximity_secs(
            candidate.primary_resets_at.as_deref(),
            now,
        ),
        scarcity_score: candidate.scarcity_score(now),
        effective_load: candidate.effective_load(),
        last_selected_at: candidate.last_selected_at.clone(),
        account_id: candidate.id,
    }
}

fn pool_route_binding_penalty_key(upstream_route_key: &str, proxy_binding_key: &str) -> String {
    format!("{upstream_route_key}\n{proxy_binding_key}")
}

fn pool_route_binding_failure_is_penalized(status: &str, failure_kind: Option<&str>) -> bool {
    status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
        && matches!(
            failure_kind,
            Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
                | Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
                | Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        )
}

async fn load_recent_route_binding_failure_penalties(
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

async fn route_binding_keys_for_candidate_scope(
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
    }
}

async fn route_binding_failure_penalty_for_account(
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

async fn build_assigned_blocked_account(
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

async fn evaluate_live_pool_candidate(
    state: &AppState,
    row: &UpstreamAccountRow,
    candidate: &AccountRoutingCandidateRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: &UpstreamAccountGroupMetadata,
    node_shunt_assignments: &mut UpstreamAccountNodeShuntAssignments,
    routing_source: PoolRoutingSelectionSource,
    now: DateTime<Utc>,
) -> Result<LivePoolCandidateEvaluation> {
    let build_evaluation =
        |eligibility, dispatch_state, resolved_account, assigned_blocked, blocked_message| {
            LivePoolCandidateEvaluation {
                score: build_pool_routing_candidate_score(
                    candidate,
                    effective_rule,
                    eligibility,
                    dispatch_state,
                    group_metadata.single_account_rotation_enabled,
                    now,
                ),
                resolved_account,
                assigned_blocked,
                blocked_message,
            }
        };

    if group_metadata.node_shunt_enabled {
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
                ForwardProxyRouteScope::pinned(proxy_key.clone()),
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
                dispatch_proxy_scope,
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
        refresh_proxy_scope,
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

pub(crate) async fn resolve_pool_account_for_request(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement(
        state,
        sticky_key,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_binding_constraint(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement(
        state,
        sticky_key,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        binding_constraint,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
) -> Result<PoolAccountResolution> {
    let now = Utc::now();
    let mut tried = excluded_ids.iter().copied().collect::<HashSet<_>>();
    let mut saw_rate_limited_candidate = false;
    let mut saw_degraded_candidate = false;
    let mut saw_other_non_rate_limited_routing_candidate = false;
    let mut saw_excluded_route_candidate = false;
    let mut saw_non_required_route_candidate = false;
    let mut saw_non_routing_candidate = false;
    let mut sticky_route_excluded_by_route_key = false;
    let mut sticky_route_still_reusable = false;
    let mut sticky_source_cut_out_guard_applies = false;
    let mut sticky_route_group_proxy_blocked_message = None;
    let mut sticky_assigned_blocked = None;
    let mut group_proxy_blocked_messages = Vec::new();
    let mut node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    let route_binding_failure_penalties =
        load_recent_route_binding_failure_penalties(&state.pool).await?;
    let mut resolved_candidates = Vec::new();

    let sticky_route = if let Some(sticky_key) = sticky_key {
        load_sticky_route(&state.pool, sticky_key).await?
    } else {
        None
    };
    let sticky_source_id = sticky_route.as_ref().map(|route| route.account_id);
    let sticky_source_rule = if let Some(route) = sticky_route.as_ref() {
        Some(load_effective_routing_rule_for_account(&state.pool, route.account_id).await?)
    } else {
        None
    };
    let forced_binding_account_id = match binding_constraint {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(account_id)) => {
            Some(*account_id)
        }
        _ => None,
    };

    if let Some(route) = sticky_route.as_ref() {
        let sticky_route_is_forced_binding_target =
            forced_binding_account_id == Some(route.account_id);
        if !sticky_route_is_forced_binding_target
            && binding_constraint.is_none()
            && tried.contains(&route.account_id)
            && sticky_source_rule
                .as_ref()
                .is_some_and(|rule| !rule.allow_cut_out)
            && load_upstream_account_row(&state.pool, route.account_id)
                .await?
                .is_some()
        {
            sticky_source_cut_out_guard_applies = true;
        }
        if !sticky_route_is_forced_binding_target
            && !tried.contains(&route.account_id)
            && let Some(row) = load_upstream_account_row(&state.pool, route.account_id).await?
        {
            tried.insert(route.account_id);
            let sticky_route_matches_binding =
                binding_constraint.is_none_or(|constraint| constraint.accepts_row(&row));
            let sticky_candidate =
                load_account_routing_candidate(&state.pool, route.account_id).await?;
            let sticky_snapshot_exhausted = sticky_candidate
                .as_ref()
                .is_some_and(routing_candidate_snapshot_is_exhausted);
            let sticky_route_key = resolve_pool_account_upstream_base_url(
                &row,
                &state.config.openai_upstream_base_url,
            )
            .ok()
            .map(|url| canonical_pool_upstream_route_key(&url));
            let sticky_route_matches_required =
                required_upstream_route_key.is_none_or(|required| {
                    sticky_route_key
                        .as_deref()
                        .is_some_and(|route_key| route_key == required)
                });
            if binding_constraint.is_none()
                && sticky_source_rule
                    .as_ref()
                    .is_some_and(|rule| !rule.allow_cut_out)
            {
                sticky_source_cut_out_guard_applies = true;
            }
            let sticky_route_is_excluded_by_route_key = sticky_route_key
                .as_deref()
                .is_some_and(|route_key| excluded_upstream_route_keys.contains(route_key));
            if !sticky_route_matches_binding {
                if is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted)
                    || is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                    || is_routing_eligible_account(&row)
                {
                    saw_other_non_rate_limited_routing_candidate = true;
                } else if is_pool_account_routing_candidate(&row) {
                    saw_non_routing_candidate = true;
                }
            } else if !sticky_route_matches_required {
                if is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted)
                    || is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                    || is_routing_eligible_account(&row)
                {
                    saw_non_required_route_candidate = true;
                } else if is_pool_account_routing_candidate(&row) {
                    saw_non_routing_candidate = true;
                }
            } else if is_account_selectable_for_sticky_reuse(&row, sticky_snapshot_exhausted, now) {
                sticky_route_still_reusable = true;
                let mut sticky_route_was_excluded = false;
                match resolve_pool_account_group_proxy_routing_readiness(
                    state,
                    row.group_name.as_deref(),
                )
                .await?
                {
                    PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => {
                        let mut evaluation = evaluate_live_pool_candidate(
                            state,
                            &row,
                            sticky_candidate
                                .as_ref()
                                .unwrap_or(&AccountRoutingCandidateRow {
                                    id: row.id,
                                    plan_type: None,
                                    secondary_used_percent: None,
                                    secondary_window_minutes: None,
                                    secondary_resets_at: None,
                                    primary_used_percent: None,
                                    primary_window_minutes: None,
                                    primary_resets_at: None,
                                    local_primary_limit: None,
                                    local_secondary_limit: None,
                                    credits_has_credits: None,
                                    credits_unlimited: None,
                                    credits_balance: None,
                                    last_selected_at: row.last_selected_at.clone(),
                                    active_sticky_conversations: 0,
                                    in_flight_reservations: 0,
                                }),
                            sticky_source_rule
                                .as_ref()
                                .expect("sticky source rule should be loaded"),
                            &group_metadata,
                            &mut node_shunt_assignments,
                            PoolRoutingSelectionSource::StickyReuse,
                            now,
                        )
                        .await?;
                        if let Some(mut account) = evaluation.resolved_account.take() {
                            account.routing_source = PoolRoutingSelectionSource::StickyReuse;
                            if !excluded_upstream_route_keys.contains(&account.upstream_route_key())
                            {
                                let route_binding_failure_penalty =
                                    route_binding_failure_penalty_for_account(
                                        state,
                                        &account,
                                        &route_binding_failure_penalties,
                                    )
                                    .await;
                                if route_binding_failure_penalty > 0 {
                                    evaluation.score.route_binding_failure_penalty =
                                        route_binding_failure_penalty;
                                    evaluation.resolved_account = Some(account);
                                    resolved_candidates.push(evaluation);
                                    sticky_route_was_excluded = true;
                                    saw_other_non_rate_limited_routing_candidate = true;
                                } else {
                                    return Ok(PoolAccountResolution::Resolved(account));
                                }
                            } else {
                                sticky_route_excluded_by_route_key = true;
                                sticky_route_was_excluded = true;
                                if is_account_degraded_for_routing(
                                    &row,
                                    sticky_snapshot_exhausted,
                                    now,
                                ) {
                                    saw_degraded_candidate = true;
                                } else {
                                    saw_excluded_route_candidate = true;
                                }
                            }
                        } else if sticky_route_is_excluded_by_route_key {
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                            saw_excluded_route_candidate = true;
                        } else {
                            if let Some(assigned_blocked) = evaluation.assigned_blocked {
                                sticky_assigned_blocked = Some(assigned_blocked.clone());
                            }
                            if let Some(message) = evaluation.blocked_message {
                                sticky_route_group_proxy_blocked_message = Some(message.clone());
                                group_proxy_blocked_messages.push(message);
                            }
                        }
                    }
                    PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                        if sticky_route_is_excluded_by_route_key {
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                            saw_excluded_route_candidate = true;
                        } else {
                            sticky_route_group_proxy_blocked_message = Some(message.clone());
                            group_proxy_blocked_messages.push(message.clone());
                            sticky_assigned_blocked = build_assigned_blocked_account(
                                state,
                                &row,
                                sticky_source_rule
                                    .as_ref()
                                    .expect("sticky source rule should be loaded"),
                                UpstreamAccountGroupMetadata::default(),
                                PoolRoutingSelectionSource::StickyReuse,
                                message,
                            )
                            .await?;
                        }
                    }
                }
                if !sticky_route_was_excluded && sticky_route_group_proxy_blocked_message.is_none()
                {
                    if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                        saw_degraded_candidate = true;
                    } else {
                        saw_other_non_rate_limited_routing_candidate = true;
                    }
                }
            } else if sticky_route_is_excluded_by_route_key
                && (is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted)
                    || is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                    || is_routing_eligible_account(&row))
            {
                saw_excluded_route_candidate = true;
            } else if is_account_rate_limited_for_routing(&row, sticky_snapshot_exhausted) {
                saw_rate_limited_candidate = true;
            } else if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                saw_degraded_candidate = true;
            } else if is_routing_eligible_account(&row) {
                saw_other_non_rate_limited_routing_candidate = true;
            } else if is_pool_account_routing_candidate(&row) {
                saw_non_routing_candidate = true;
            }
        }
        if sticky_source_cut_out_guard_applies {
            if let Some(assigned_blocked) = sticky_assigned_blocked {
                return Ok(PoolAccountResolution::AssignedBlocked(assigned_blocked));
            }
            if let Some(message) = sticky_route_group_proxy_blocked_message {
                return Ok(PoolAccountResolution::BlockedByPolicy(message));
            }
            let message =
                "sticky conversation cannot cut out of the current account because routing policy forbids it"
                    .to_string();
            if let Some(row) = load_upstream_account_row(&state.pool, route.account_id).await?
                && let Some(assigned_blocked) = build_assigned_blocked_account(
                    state,
                    &row,
                    sticky_source_rule
                        .as_ref()
                        .expect("sticky source rule should be loaded"),
                    UpstreamAccountGroupMetadata::default(),
                    PoolRoutingSelectionSource::StickyReuse,
                    message.clone(),
                )
                .await?
            {
                return Ok(PoolAccountResolution::AssignedBlocked(assigned_blocked));
            }
            return Ok(PoolAccountResolution::BlockedByPolicy(message));
        }
    }

    let mut candidates = load_account_routing_candidates(&state.pool, &tried).await?;
    for candidate in &mut candidates {
        candidate.in_flight_reservations = pool_routing_reservation_count(state, candidate.id);
        if forced_binding_account_id == Some(candidate.id) && sticky_source_id == Some(candidate.id)
        {
            candidate.active_sticky_conversations =
                candidate.active_sticky_conversations.saturating_sub(1);
        }
    }
    let candidate_effective_rules = load_effective_routing_rules_for_accounts(
        &state.pool,
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
    )
    .await?;
    for candidate in candidates {
        let Some(row) = load_upstream_account_row(&state.pool, candidate.id).await? else {
            continue;
        };
        if binding_constraint.is_some_and(|constraint| !constraint.accepts_row(&row)) {
            if is_pool_account_routing_candidate(&row) {
                saw_other_non_rate_limited_routing_candidate = true;
            }
            continue;
        }
        let snapshot_exhausted = routing_candidate_snapshot_is_exhausted(&candidate);
        let candidate_route_key =
            resolve_pool_account_upstream_base_url(&row, &state.config.openai_upstream_base_url)
                .ok()
                .map(|url| canonical_pool_upstream_route_key(&url));
        let candidate_route_matches_required = required_upstream_route_key.is_none_or(|required| {
            candidate_route_key
                .as_deref()
                .is_some_and(|route_key| route_key == required)
        });
        let candidate_route_is_excluded_by_route_key = candidate_route_key
            .as_deref()
            .is_some_and(|route_key| excluded_upstream_route_keys.contains(route_key));
        if !candidate_route_matches_required {
            if is_account_rate_limited_for_routing(&row, snapshot_exhausted)
                || is_account_degraded_for_routing(&row, snapshot_exhausted, now)
                || is_routing_eligible_account(&row)
            {
                saw_non_required_route_candidate = true;
            } else {
                saw_non_routing_candidate = true;
            }
            continue;
        }
        if candidate_route_is_excluded_by_route_key {
            if is_account_rate_limited_for_routing(&row, snapshot_exhausted)
                || is_account_degraded_for_routing(&row, snapshot_exhausted, now)
                || is_routing_eligible_account(&row)
            {
                saw_excluded_route_candidate = true;
            } else {
                saw_non_routing_candidate = true;
            }
            continue;
        }
        if !is_account_selectable_for_fresh_assignment(&row, snapshot_exhausted, now) {
            if is_account_rate_limited_for_routing(&row, snapshot_exhausted) {
                saw_rate_limited_candidate = true;
            } else if is_account_degraded_for_routing(&row, snapshot_exhausted, now) {
                saw_degraded_candidate = true;
            } else if is_routing_eligible_account(&row) {
                saw_other_non_rate_limited_routing_candidate = true;
            } else {
                saw_non_routing_candidate = true;
            }
            continue;
        }
        let Some(effective_rule) = candidate_effective_rules.get(&row.id) else {
            continue;
        };
        if !account_accepts_concurrency_limit(
            candidate.effective_load(),
            PoolRoutingSelectionSource::FreshAssignment,
            effective_rule,
        ) {
            saw_other_non_rate_limited_routing_candidate = true;
            continue;
        }
        if !account_accepts_sticky_assignment(
            &state.pool,
            row.id,
            sticky_key,
            sticky_source_id,
            effective_rule,
            forced_binding_account_id == Some(row.id),
        )
        .await?
        {
            saw_other_non_rate_limited_routing_candidate = true;
            continue;
        }
        let group_metadata = match resolve_pool_account_group_proxy_routing_readiness(
            state,
            row.group_name.as_deref(),
        )
        .await?
        {
            PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => group_metadata,
            PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                group_proxy_blocked_messages.push(message);
                continue;
            }
        };
        let mut evaluation = evaluate_live_pool_candidate(
            state,
            &row,
            &candidate,
            effective_rule,
            &group_metadata,
            &mut node_shunt_assignments,
            PoolRoutingSelectionSource::FreshAssignment,
            now,
        )
        .await?;
        match evaluation.score.eligibility {
            PoolRoutingCandidateEligibility::Assignable
            | PoolRoutingCandidateEligibility::SoftDegraded
                if evaluation.resolved_account.is_some() =>
            {
                if let Some(account) = evaluation.resolved_account.as_ref() {
                    evaluation.score.route_binding_failure_penalty =
                        route_binding_failure_penalty_for_account(
                            state,
                            account,
                            &route_binding_failure_penalties,
                        )
                        .await;
                }
                resolved_candidates.push(evaluation);
            }
            PoolRoutingCandidateEligibility::HardBlocked => {
                if let Some(message) = evaluation.blocked_message {
                    group_proxy_blocked_messages.push(message);
                } else {
                    saw_other_non_rate_limited_routing_candidate = true;
                }
            }
            _ => {
                saw_other_non_rate_limited_routing_candidate = true;
            }
        }
    }

    resolved_candidates
        .sort_by(|lhs, rhs| compare_pool_routing_candidate_scores(&lhs.score, &rhs.score));
    for evaluation in resolved_candidates {
        if let Some(account) = evaluation.resolved_account {
            return Ok(PoolAccountResolution::Resolved(account));
        }
    }

    if sticky_route_still_reusable
        && !sticky_route_excluded_by_route_key
        && let Some(assigned_blocked) = sticky_assigned_blocked
    {
        return Ok(PoolAccountResolution::AssignedBlocked(assigned_blocked));
    }

    if !saw_other_non_rate_limited_routing_candidate
        && let Some(message) =
            summarize_pool_group_proxy_blocked_messages(&group_proxy_blocked_messages)
    {
        return Ok(PoolAccountResolution::BlockedByPolicy(message));
    }
    if saw_rate_limited_candidate
        && !saw_degraded_candidate
        && !saw_other_non_rate_limited_routing_candidate
        && !saw_excluded_route_candidate
    {
        return Ok(PoolAccountResolution::RateLimited);
    }
    if saw_degraded_candidate
        && !saw_rate_limited_candidate
        && !saw_other_non_rate_limited_routing_candidate
        && !saw_excluded_route_candidate
        && !saw_non_routing_candidate
    {
        return Ok(PoolAccountResolution::DegradedOnly);
    }
    if saw_other_non_rate_limited_routing_candidate
        || saw_non_required_route_candidate
        || saw_excluded_route_candidate
        || saw_non_routing_candidate
        || (saw_rate_limited_candidate && saw_degraded_candidate)
    {
        return Ok(PoolAccountResolution::Unavailable);
    }

    Ok(PoolAccountResolution::NoCandidate)
}
