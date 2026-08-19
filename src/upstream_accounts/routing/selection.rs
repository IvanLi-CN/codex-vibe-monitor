use super::*;

#[derive(Debug, Clone)]
pub(crate) struct LivePoolCandidateEvaluation {
    score: PoolRoutingCandidateScore,
    resolved_account: Option<PoolResolvedAccount>,
    assigned_blocked: Option<PoolAssignedBlockedAccount>,
    blocked_message: Option<String>,
}

pub(crate) const POOL_ROUTE_BINDING_FAILURE_PENALTY_WINDOW_SECS: i64 = 300;
const POOL_ROUTING_SELECTION_AUDIT_EXCLUSION_LIMIT: usize = 12;

fn no_candidate_audit(
    terminal_reason_code: &str,
    candidate_count: usize,
    eligible_candidate_count: usize,
    reservation_conflict_count: usize,
    exclusions: &[PoolRoutingSelectionAuditExcludedCandidate],
) -> PoolRoutingNoCandidateAudit {
    let mut excluded_reason_counts = std::collections::BTreeMap::new();
    for candidate in exclusions {
        *excluded_reason_counts
            .entry(candidate.reason_code.clone())
            .or_insert(0) += 1;
    }
    PoolRoutingNoCandidateAudit {
        terminal_reason_code: terminal_reason_code.to_string(),
        candidate_count,
        eligible_candidate_count,
        reservation_conflict_count,
        next_eligible_at: None,
        excluded_reason_counts,
        candidates: exclusions
            .iter()
            .take(POOL_ROUTING_SELECTION_AUDIT_EXCLUSION_LIMIT)
            .map(|candidate| PoolRoutingNoCandidateAuditCandidate {
                account_id: candidate.account_id,
                account_name: candidate.account_name.clone(),
                reason_code: candidate.reason_code.clone(),
            })
            .collect(),
    }
}

fn no_candidate_audit_from_snapshot(
    snapshot: &PoolRoutingSnapshot,
    requested_model: Option<&str>,
    terminal_reason_code: &str,
    candidate_count: usize,
    eligible_candidate_count: usize,
    reservation_conflict_count: usize,
    exclusions: &[PoolRoutingSelectionAuditExcludedCandidate],
) -> PoolRoutingNoCandidateAudit {
    let mut audit = no_candidate_audit(
        terminal_reason_code,
        candidate_count,
        eligible_candidate_count,
        reservation_conflict_count,
        exclusions,
    );
    let cooldown_candidate_ids = exclusions
        .iter()
        .filter(|candidate| candidate.reason_code == "modelTemporarilyExcluded")
        .map(|candidate| candidate.account_id)
        .collect::<Vec<_>>();
    audit.next_eligible_at = snapshot.earliest_model_route_cooldown_expiry_for_accounts(
        requested_model,
        &cooldown_candidate_ids,
    );
    audit
}

fn no_candidate_audit_with_reservation_conflict(
    account: &PoolResolvedAccount,
    sticky: bool,
) -> PoolRoutingNoCandidateAudit {
    let reason_code = if sticky {
        "stickyRouteReservationConflict"
    } else {
        "modelConcurrencyLimit"
    };
    let exclusion = PoolRoutingSelectionAuditExcludedCandidate {
        account_id: account.account_id,
        account_name: account.display_name.clone(),
        reason_code: reason_code.to_string(),
    };
    no_candidate_audit(reason_code, 1, 1, 1, &[exclusion])
}

fn model_route_penalty_code(score: u8) -> &'static str {
    match score {
        0 => "normal",
        1 => "demoted",
        2 => "excluded",
        _ => "unknown",
    }
}

fn routing_selection_score_snapshot(
    score: &PoolRoutingCandidateScore,
) -> PoolRoutingSelectionScoreSnapshot {
    PoolRoutingSelectionScoreSnapshot {
        eligibility: score.eligibility.as_persisted_str().to_string(),
        route_binding_failure_penalty: score.route_binding_failure_penalty,
        model_route_penalty: score.model_route_penalty,
        model_route_penalty_code: model_route_penalty_code(score.model_route_penalty).to_string(),
        routing_priority_rank: score.routing_priority_rank,
        capacity_lane: score.capacity_lane.as_persisted_str().to_string(),
        dispatch_state: score.dispatch_state.as_persisted_str().to_string(),
        secondary_reset_proximity_secs: score.secondary_reset_proximity_secs,
        primary_reset_proximity_secs: score.primary_reset_proximity_secs,
        scarcity_score: format!("{:.6}", score.scarcity_score),
        effective_load: score.effective_load,
        last_selected_at: score.last_selected_at.clone(),
    }
}

fn push_routing_selection_audit_exclusion(
    exclusions: &mut Vec<PoolRoutingSelectionAuditExcludedCandidate>,
    row: &UpstreamAccountRow,
    reason_code: &str,
) {
    if exclusions
        .iter()
        .any(|candidate| candidate.account_id == row.id)
    {
        return;
    }
    exclusions.push(PoolRoutingSelectionAuditExcludedCandidate {
        account_id: row.id,
        account_name: row.display_name.clone(),
        reason_code: reason_code.to_string(),
    });
}

fn pool_routing_selection_winner_reason(
    winner: &PoolRoutingCandidateScore,
    runner_up: Option<&PoolRoutingCandidateScore>,
) -> &'static str {
    let Some(runner_up) = runner_up else {
        return "onlyEligibleCandidate";
    };
    let winner_requires_retry_original =
        winner.dispatch_state == PoolRoutingCandidateDispatchState::RetryOriginalNode;
    let runner_up_requires_retry_original =
        runner_up.dispatch_state == PoolRoutingCandidateDispatchState::RetryOriginalNode;
    if winner_requires_retry_original != runner_up_requires_retry_original {
        return "avoidsRetryOriginalNode";
    }
    if winner.capacity_lane != runner_up.capacity_lane {
        return "lowerCapacityLane";
    }
    if winner.route_binding_failure_penalty != runner_up.route_binding_failure_penalty {
        return "lowerRouteBindingFailurePenalty";
    }
    if winner.model_route_penalty != runner_up.model_route_penalty {
        return "lowerModelRoutePenalty";
    }
    if winner.routing_priority_rank != runner_up.routing_priority_rank {
        return "higherRoutingPriority";
    }
    if winner.eligibility != runner_up.eligibility {
        return "higherEligibility";
    }
    if winner.dispatch_state != runner_up.dispatch_state {
        return "preferredDispatchState";
    }
    if compare_reset_proximity_for_rotation_candidates(
        winner.single_account_rotation_enabled,
        winner.secondary_reset_proximity_secs,
        runner_up.single_account_rotation_enabled,
        runner_up.secondary_reset_proximity_secs,
    )
    .is_ne()
    {
        return "secondaryResetProximity";
    }
    if compare_reset_proximity_for_rotation_candidates(
        winner.single_account_rotation_enabled,
        winner.primary_reset_proximity_secs,
        runner_up.single_account_rotation_enabled,
        runner_up.primary_reset_proximity_secs,
    )
    .is_ne()
    {
        return "primaryResetProximity";
    }
    if winner
        .scarcity_score
        .total_cmp(&runner_up.scarcity_score)
        .is_ne()
    {
        return "lowerScarcity";
    }
    if winner.effective_load != runner_up.effective_load {
        return "lowerEffectiveLoad";
    }
    if winner.last_selected_at != runner_up.last_selected_at {
        return "leastRecentlySelected";
    }
    "stableAccountOrder"
}

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
                        .then_with(|| lhs.model_route_penalty.cmp(&rhs.model_route_penalty))
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

pub(crate) fn compare_reset_proximity_for_rotation_candidates(
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

pub(crate) fn compare_optional_reset_proximity(
    lhs: Option<i64>,
    rhs: Option<i64>,
) -> std::cmp::Ordering {
    match (lhs, rhs) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

pub(crate) fn reset_proximity_secs(resets_at: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    resets_at
        .and_then(parse_rfc3339_utc)
        .map(|reset| reset.signed_duration_since(now).num_seconds().abs())
}

pub(crate) fn build_pool_routing_candidate_score(
    candidate: &AccountRoutingCandidateRow,
    effective_rule: &EffectiveRoutingRule,
    eligibility: PoolRoutingCandidateEligibility,
    dispatch_state: PoolRoutingCandidateDispatchState,
    single_account_rotation_enabled: bool,
    runtime_last_selected_at: Option<String>,
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
        model_route_penalty: 0,
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
        last_selected_at: runtime_last_selected_at.or_else(|| candidate.last_selected_at.clone()),
        account_id: candidate.id,
    }
}

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

pub(crate) async fn resolve_pool_account_for_request(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_internal(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        None,
        None,
        "",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_image_intent(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_image_intent_and_codex_imagegen_request(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        endpoint,
        image_intent,
        false,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_image_intent_and_codex_imagegen_request(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        None,
        None,
        endpoint,
        image_intent,
        codex_imagegen_request,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_binding_constraint(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_internal(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        binding_constraint,
        None,
        "",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_binding_constraint_and_image_intent(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_and_image_intent(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        binding_constraint,
        endpoint,
        image_intent,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_internal(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        None,
        "",
        crate::ImageIntent::Unknown,
        false,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement_and_image_intent(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        None,
        endpoint,
        image_intent,
        false,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        conversation_override,
        endpoint,
        image_intent,
        false,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
) -> Result<PoolAccountResolution> {
    resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        binding_constraint,
        conversation_override,
        endpoint,
        image_intent,
        codex_imagegen_request,
        None,
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
    reservation_key: Option<&str>,
) -> Result<PoolAccountResolution> {
    // This selection state machine is intentionally large. Keep it off callers'
    // async task stacks, including the live-first request reader.
    Box::pin(
        resolve_pool_account_for_request_with_route_requirement_internal(
            state,
            sticky_key,
            requested_model,
            excluded_ids,
            excluded_upstream_route_keys,
            required_upstream_route_key,
            binding_constraint,
            conversation_override,
            endpoint,
            image_intent,
            codex_imagegen_request,
            reservation_key,
        ),
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement_internal(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    conversation_override: Option<&ConversationRoutingOverride>,
    endpoint: &str,
    image_intent: crate::ImageIntent,
    codex_imagegen_request: bool,
    reservation_key: Option<&str>,
) -> Result<PoolAccountResolution> {
    let Some(routing_snapshot) = state.pool_routing_snapshot.current() else {
        // A routing request must never turn a cold snapshot into an on-demand
        // SQLite read. The background reconciler owns recovery.
        return Ok(PoolAccountResolution::NoCandidate(
            PoolRoutingNoCandidateAudit::no_eligible(),
        ));
    };
    let now = Utc::now();
    let mut tried = excluded_ids.iter().copied().collect::<HashSet<_>>();
    let mut saw_rate_limited_candidate = false;
    let mut saw_degraded_candidate = false;
    let mut saw_other_non_rate_limited_routing_candidate = false;
    let mut saw_excluded_route_candidate = false;
    let mut saw_non_required_route_candidate = false;
    let mut saw_non_routing_candidate = false;
    let mut saw_model_concurrency_limited_candidate = false;
    let mut sticky_route_excluded_by_route_key = false;
    let mut sticky_route_still_reusable = false;
    let mut sticky_source_cut_out_guard_applies = false;
    let mut sticky_route_group_proxy_blocked_message = None;
    let mut sticky_assigned_blocked = None;
    let mut group_proxy_blocked_messages = Vec::new();
    let mut node_shunt_assignments = routing_snapshot.node_shunt_assignments();
    let route_binding_failure_penalties = routing_snapshot.route_binding_failure_penalties();
    let mut resolved_candidates = Vec::new();
    let mut sticky_queue_reservation_conflict: Option<(i64, String)> = None;
    let (sticky_route, sticky_affinity_generation) = if let Some(sticky_key) = sticky_key {
        let (route, generation) =
            routing_snapshot.sticky_route_with_model_generation(sticky_key, requested_model);
        (route, Some(generation))
    } else {
        (None, None)
    };
    let sticky_source_id = sticky_route.as_ref().map(|route| route.account_id);
    let sticky_model_penalties = routing_snapshot.model_route_penalties(
        &sticky_source_id.into_iter().collect::<Vec<_>>(),
        requested_model,
    );
    let sticky_source_rule = if let Some(route) = sticky_route.as_ref() {
        let mut rule = routing_snapshot
            .effective_rule(route.account_id)
            .cloned()
            .unwrap_or_else(|| build_effective_routing_rule(&[]));
        apply_conversation_routing_override(&mut rule, conversation_override);
        Some(rule)
    } else {
        None
    };
    let sticky_cut_out_blocked_by_policy =
        match conversation_override.and_then(|policy| policy.allow_switch_upstream) {
            Some(true) => false,
            Some(false) => true,
            None => sticky_source_rule
                .as_ref()
                .is_some_and(|rule| !rule.allow_cut_out),
        };
    let forced_binding_account_id = match binding_constraint {
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(account_id)) => {
            Some(*account_id)
        }
        _ => None,
    };
    let non_explicit_sticky_escape_enabled = !matches!(
        binding_constraint,
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(_))
    );
    let sticky_source_transport_decode_escape = if non_explicit_sticky_escape_enabled {
        if let Some(account_id) = sticky_source_id {
            routing_snapshot
                .transport_decode_sticky_escape_states(&[account_id])
                .contains_key(&account_id)
        } else {
            false
        }
    } else {
        false
    };
    let sticky_fallback_handoff_policy_enabled = sticky_source_rule
        .as_ref()
        .is_some_and(|rule| rule.priority_tier == TagPriorityTier::Fallback)
        && binding_constraint.is_none()
        && !sticky_source_transport_decode_escape
        && !sticky_cut_out_blocked_by_policy;
    let mut sticky_fallback_handoff_enabled = false;
    let bypass_requested_model_filter = binding_constraint.is_some();
    let conversation_available_models_override = conversation_override.is_some_and(|policy| {
        policy.available_models.is_some()
            || policy.available_models_mode.is_some()
            || policy.available_models_invalid
    });

    if let Some(route) = sticky_route.as_ref() {
        let sticky_route_is_forced_binding_target =
            forced_binding_account_id == Some(route.account_id);
        if !sticky_route_is_forced_binding_target
            && binding_constraint.is_none()
            && !sticky_source_transport_decode_escape
            && sticky_cut_out_blocked_by_policy
            && tried.contains(&route.account_id)
            && routing_snapshot.account(route.account_id).is_some()
        {
            sticky_source_cut_out_guard_applies = true;
        }
        if !sticky_route_is_forced_binding_target
            && !tried.contains(&route.account_id)
            && let Some(row) = routing_snapshot.account(route.account_id).cloned()
        {
            tried.insert(route.account_id);
            let sticky_route_matches_binding =
                binding_constraint.is_none_or(|constraint| constraint.accepts_row(&row));
            let sticky_candidate = routing_snapshot.candidate(route.account_id).cloned();
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
                && !sticky_source_transport_decode_escape
                && sticky_cut_out_blocked_by_policy
            {
                sticky_source_cut_out_guard_applies = true;
            }
            let sticky_route_is_excluded_by_route_key = sticky_route_key
                .as_deref()
                .is_some_and(|route_key| excluded_upstream_route_keys.contains(route_key));
            let sticky_model_penalty = if sticky_route_matches_binding
                && sticky_route_matches_required
                && !sticky_source_transport_decode_escape
            {
                sticky_model_penalties
                    .get(&row.id)
                    .copied()
                    .unwrap_or(ModelRoutePenalty::Normal)
            } else {
                ModelRoutePenalty::Normal
            };
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
            } else if sticky_source_transport_decode_escape {
                saw_degraded_candidate = true;
            } else if sticky_model_penalty == ModelRoutePenalty::Excluded {
                // A sticky account whose requested model is cooling down is still a
                // model-level degraded candidate, not an unavailable account.
                saw_degraded_candidate = true;
            } else if is_account_selectable_for_sticky_reuse(&row, sticky_snapshot_exhausted, now) {
                let sticky_model_accepted = match sticky_source_rule.as_ref() {
                    None => true,
                    Some(rule)
                        if bypass_requested_model_filter
                            && !conversation_available_models_override =>
                    {
                        account_accepts_requested_model_or_mapping_with_available_models_bypass(
                            state,
                            row.id,
                            requested_model,
                            rule,
                        )
                        .await?
                    }
                    // Conversation-scoped model policy is an explicit caller constraint.
                    // A local account mapping can bypass its ordinary availability policy,
                    // but must not broaden this per-conversation restriction.
                    Some(rule) if conversation_available_models_override => {
                        account_accepts_requested_model(requested_model, rule)
                    }
                    Some(rule) => {
                        account_accepts_requested_model_or_cached_mapping(
                            state,
                            row.id,
                            requested_model,
                            rule,
                        )
                        .await?
                    }
                };
                if sticky_model_accepted
                    && account_accepts_request_capabilities(
                        request_capability_requirements_after_codex_imagegen_rewrite(
                            endpoint,
                            image_intent,
                            requested_model,
                            codex_imagegen_request,
                            sticky_source_rule
                                .as_ref()
                                .expect("sticky source rule should be loaded"),
                        ),
                        effective_capability_support(
                            decode_capability_support(row.response_endpoint_capability.as_deref()),
                            decode_capability_override(
                                row.policy_response_endpoint_capability_override.as_deref(),
                            ),
                        ),
                        effective_capability_support(
                            decode_capability_support(row.chat_completions_capability.as_deref()),
                            decode_capability_override(
                                row.policy_chat_completions_capability_override.as_deref(),
                            ),
                        ),
                        effective_capability_support(
                            decode_capability_support(row.image_endpoint_capability.as_deref()),
                            decode_capability_override(
                                row.policy_image_endpoint_capability_override.as_deref(),
                            ),
                        ),
                        effective_capability_support(
                            decode_capability_support(
                                row.response_image_tool_capability.as_deref(),
                            ),
                            decode_capability_override(
                                row.policy_response_image_tool_capability_override
                                    .as_deref(),
                            ),
                        ),
                        effective_capability_support(
                            decode_capability_support(row.codex_imagegen_capability.as_deref()),
                            decode_capability_override(
                                row.policy_codex_imagegen_capability_override.as_deref(),
                            ),
                        ),
                        if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
                            effective_capability_support(
                                decode_capability_support(
                                    row.standalone_search_capability.as_deref(),
                                ),
                                decode_capability_override(
                                    row.policy_standalone_search_capability_override.as_deref(),
                                ),
                            )
                        } else {
                            CapabilitySupport::Supported
                        },
                    )
                {
                    sticky_route_still_reusable = true;
                    let mut sticky_route_was_excluded = false;
                    let group_readiness = if row.bound_proxy_keys().is_empty() {
                        resolve_pool_account_group_proxy_routing_readiness_with_metadata(
                            state,
                            row.group_name.as_deref(),
                            routing_snapshot.group_metadata(row.group_name.as_deref()),
                        )
                        .await?
                    } else {
                        PoolAccountGroupProxyRoutingReadiness::Ready(
                            routing_snapshot.group_metadata(row.group_name.as_deref()),
                        )
                    };
                    match group_readiness {
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
                                conversation_override,
                                now,
                            )
                            .await?;
                            evaluation.score.model_route_penalty =
                                sticky_model_penalty.score() as u8;
                            if let Some(mut account) = evaluation.resolved_account.take() {
                                account.routing_source = PoolRoutingSelectionSource::StickyReuse;
                                let account = account
                                    .with_sticky_affinity_generation(sticky_affinity_generation);
                                if !excluded_upstream_route_keys
                                    .contains(&account.upstream_route_key())
                                {
                                    let route_binding_failure_penalty =
                                        route_binding_failure_penalty_for_account(
                                            state,
                                            &account,
                                            route_binding_failure_penalties,
                                        )
                                        .await;
                                    sticky_fallback_handoff_enabled =
                                        sticky_fallback_handoff_policy_enabled
                                            && route_binding_failure_penalty == 0
                                            && sticky_model_penalty == ModelRoutePenalty::Normal;
                                    if route_binding_failure_penalty > 0
                                        || sticky_model_penalty != ModelRoutePenalty::Normal
                                        || sticky_fallback_handoff_policy_enabled
                                    {
                                        if sticky_source_cut_out_guard_applies {
                                            if reserve_sticky_model_route(
                                                state,
                                                reservation_key,
                                                &account,
                                                requested_model,
                                            )
                                            .await?
                                            {
                                                return Ok(PoolAccountResolution::Resolved(
                                                    account,
                                                ));
                                            }
                                            let reason_code = if routing_snapshot
                                                .model_route_requires_expired_cooldown_probe(
                                                    account.account_id,
                                                    requested_model,
                                                ) {
                                                "expiredCooldownProbe"
                                            } else {
                                                "stickyRouteReservationConflict"
                                            };
                                            sticky_queue_reservation_conflict =
                                                Some((account.account_id, reason_code.to_string()));
                                        }
                                        evaluation.score.route_binding_failure_penalty =
                                            route_binding_failure_penalty;
                                        evaluation.resolved_account = Some(account);
                                        resolved_candidates.push(evaluation);
                                        sticky_route_was_excluded = route_binding_failure_penalty
                                            > 0
                                            || sticky_model_penalty != ModelRoutePenalty::Normal;
                                        saw_other_non_rate_limited_routing_candidate = true;
                                    } else {
                                        if reserve_sticky_model_route(
                                            state,
                                            reservation_key,
                                            &account,
                                            requested_model,
                                        )
                                        .await?
                                        {
                                            return Ok(PoolAccountResolution::Resolved(account));
                                        }
                                        let cache_hit_protection =
                                            routing_snapshot.cache_hit_protection();
                                        if cache_hit_protection.overflow_mode
                                            == CacheHitOverflowMode::Queue
                                        {
                                            let reason_code = if routing_snapshot
                                                .model_route_requires_expired_cooldown_probe(
                                                    account.account_id,
                                                    requested_model,
                                                ) {
                                                "expiredCooldownProbe"
                                            } else {
                                                "stickyRouteReservationConflict"
                                            };
                                            sticky_queue_reservation_conflict =
                                                Some((account.account_id, reason_code.to_string()));
                                        }
                                        // A normal sticky route may be handed off when reroute is
                                        // selected. Preserve it as a scored candidate so the common
                                        // loop below keeps the atomic cap check for any retry.
                                        evaluation.resolved_account = Some(account);
                                        resolved_candidates.push(evaluation);
                                        saw_model_concurrency_limited_candidate = true;
                                        saw_other_non_rate_limited_routing_candidate = true;
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
                                    sticky_route_group_proxy_blocked_message =
                                        Some(message.clone());
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
                    if !sticky_route_was_excluded
                        && sticky_route_group_proxy_blocked_message.is_none()
                    {
                        if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                            saw_degraded_candidate = true;
                        } else {
                            saw_other_non_rate_limited_routing_candidate = true;
                        }
                    }
                } else {
                    saw_other_non_rate_limited_routing_candidate = true;
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
            if let Some(row) = routing_snapshot.account(route.account_id).cloned()
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

    let mut selection_audit_exclusions = Vec::new();
    for account_id in excluded_ids {
        if let Some(row) = routing_snapshot.account(*account_id) {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                row,
                "previousAttemptExcluded",
            );
        }
    }
    if let Some(account_id) = sticky_source_id
        && !excluded_ids.contains(&account_id)
        && tried.contains(&account_id)
        && let Some(row) = routing_snapshot.account(account_id)
    {
        push_routing_selection_audit_exclusion(
            &mut selection_audit_exclusions,
            row,
            "stickyReuseUnavailable",
        );
    }

    let mut candidates = routing_snapshot.candidates(&tried);
    let candidate_count =
        candidates.len() + usize::from(sticky_queue_reservation_conflict.is_some());
    let sticky_escape_account_states = if non_explicit_sticky_escape_enabled {
        routing_snapshot.transport_decode_sticky_escape_states(
            &candidates
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>(),
        )
    } else {
        HashMap::new()
    };
    for candidate in &mut candidates {
        candidate.in_flight_reservations = pool_routing_reservation_count(state, candidate.id);
        if forced_binding_account_id == Some(candidate.id) && sticky_source_id == Some(candidate.id)
        {
            candidate.active_sticky_conversations =
                candidate.active_sticky_conversations.saturating_sub(1);
        }
    }
    let model_route_penalties = routing_snapshot.model_route_penalties(
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
        requested_model,
    );
    let mut candidate_effective_rules = routing_snapshot.effective_rules_for(
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
    );
    for rule in candidate_effective_rules.values_mut() {
        apply_conversation_routing_override(rule, conversation_override);
    }
    for candidate in candidates {
        let Some(row) = routing_snapshot.account(candidate.id).cloned() else {
            continue;
        };
        if binding_constraint.is_some_and(|constraint| !constraint.accepts_row(&row)) {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "bindingConstraint",
            );
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
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "requiredRouteMismatch",
            );
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
        if sticky_escape_account_states.contains_key(&candidate.id) {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "recentTransportFailure",
            );
            saw_degraded_candidate = true;
            continue;
        }
        if candidate_route_is_excluded_by_route_key {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "previousAttemptExcluded",
            );
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
            let reason_code = if is_account_rate_limited_for_routing(&row, snapshot_exhausted) {
                saw_rate_limited_candidate = true;
                "rateLimited"
            } else if is_account_degraded_for_routing(&row, snapshot_exhausted, now) {
                saw_degraded_candidate = true;
                "degraded"
            } else if is_routing_eligible_account(&row) {
                saw_other_non_rate_limited_routing_candidate = true;
                "notSelectableForFreshAssignment"
            } else {
                saw_non_routing_candidate = true;
                "unavailable"
            };
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                reason_code,
            );
            continue;
        }
        let Some(effective_rule) = candidate_effective_rules.get(&row.id) else {
            continue;
        };
        if sticky_fallback_handoff_enabled
            && effective_rule.priority_tier.routing_rank()
                >= sticky_source_rule
                    .as_ref()
                    .expect("fallback sticky source rule should be loaded")
                    .priority_tier
                    .routing_rank()
        {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "notHigherPriorityThanStickyFallback",
            );
            continue;
        }
        let model_accepted =
            if bypass_requested_model_filter && !conversation_available_models_override {
                account_accepts_requested_model_or_mapping_with_available_models_bypass(
                    state,
                    row.id,
                    requested_model,
                    effective_rule,
                )
                .await?
            } else if conversation_available_models_override {
                account_accepts_requested_model(requested_model, effective_rule)
            } else {
                account_accepts_requested_model_or_cached_mapping(
                    state,
                    row.id,
                    requested_model,
                    effective_rule,
                )
                .await?
            };
        if !model_accepted {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "modelNotAllowed",
            );
            saw_other_non_rate_limited_routing_candidate = true;
            continue;
        }
        if !account_accepts_request_capabilities(
            request_capability_requirements_after_codex_imagegen_rewrite(
                endpoint,
                image_intent,
                requested_model,
                codex_imagegen_request,
                effective_rule,
            ),
            effective_capability_support(
                decode_capability_support(row.response_endpoint_capability.as_deref()),
                decode_capability_override(
                    row.policy_response_endpoint_capability_override.as_deref(),
                ),
            ),
            effective_capability_support(
                decode_capability_support(row.chat_completions_capability.as_deref()),
                decode_capability_override(
                    row.policy_chat_completions_capability_override.as_deref(),
                ),
            ),
            effective_capability_support(
                decode_capability_support(row.image_endpoint_capability.as_deref()),
                decode_capability_override(
                    row.policy_image_endpoint_capability_override.as_deref(),
                ),
            ),
            effective_capability_support(
                decode_capability_support(row.response_image_tool_capability.as_deref()),
                decode_capability_override(
                    row.policy_response_image_tool_capability_override
                        .as_deref(),
                ),
            ),
            effective_capability_support(
                decode_capability_support(row.codex_imagegen_capability.as_deref()),
                decode_capability_override(
                    row.policy_codex_imagegen_capability_override.as_deref(),
                ),
            ),
            if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
                effective_capability_support(
                    decode_capability_support(row.standalone_search_capability.as_deref()),
                    decode_capability_override(
                        row.policy_standalone_search_capability_override.as_deref(),
                    ),
                )
            } else {
                CapabilitySupport::Supported
            },
        ) {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "capabilityUnsupported",
            );
            saw_other_non_rate_limited_routing_candidate = true;
            continue;
        }
        if !account_accepts_concurrency_limit(
            candidate.effective_load(),
            PoolRoutingSelectionSource::FreshAssignment,
            effective_rule,
        ) {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "concurrencyLimit",
            );
            saw_other_non_rate_limited_routing_candidate = true;
            continue;
        }
        if !account_accepts_sticky_assignment(
            row.id,
            sticky_key,
            sticky_source_id,
            effective_rule,
            forced_binding_account_id == Some(row.id),
        ) {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "stickyPolicy",
            );
            saw_other_non_rate_limited_routing_candidate = true;
            continue;
        }
        let group_readiness = if row.bound_proxy_keys().is_empty() {
            resolve_pool_account_group_proxy_routing_readiness_with_metadata(
                state,
                row.group_name.as_deref(),
                routing_snapshot.group_metadata(row.group_name.as_deref()),
            )
            .await?
        } else {
            PoolAccountGroupProxyRoutingReadiness::Ready(
                routing_snapshot.group_metadata(row.group_name.as_deref()),
            )
        };
        let group_metadata = match group_readiness {
            PoolAccountGroupProxyRoutingReadiness::Ready(group_metadata) => group_metadata,
            PoolAccountGroupProxyRoutingReadiness::Blocked(message) => {
                push_routing_selection_audit_exclusion(
                    &mut selection_audit_exclusions,
                    &row,
                    "forwardProxyUnavailable",
                );
                group_proxy_blocked_messages.push(message);
                continue;
            }
        };
        let model_penalty = model_route_penalties
            .get(&row.id)
            .copied()
            .unwrap_or(ModelRoutePenalty::Normal);
        if model_penalty == ModelRoutePenalty::Excluded {
            push_routing_selection_audit_exclusion(
                &mut selection_audit_exclusions,
                &row,
                "modelTemporarilyExcluded",
            );
            saw_degraded_candidate = true;
            continue;
        }
        let mut evaluation = evaluate_live_pool_candidate(
            state,
            &row,
            &candidate,
            effective_rule,
            &group_metadata,
            &mut node_shunt_assignments,
            PoolRoutingSelectionSource::FreshAssignment,
            conversation_override,
            now,
        )
        .await?;
        evaluation.score.model_route_penalty = model_penalty.score() as u8;
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
                            route_binding_failure_penalties,
                        )
                        .await;
                }
                resolved_candidates.push(evaluation);
            }
            PoolRoutingCandidateEligibility::HardBlocked => {
                push_routing_selection_audit_exclusion(
                    &mut selection_audit_exclusions,
                    &row,
                    "forwardProxyUnavailable",
                );
                if let Some(message) = evaluation.blocked_message {
                    group_proxy_blocked_messages.push(message);
                } else {
                    saw_other_non_rate_limited_routing_candidate = true;
                }
            }
            _ => {
                push_routing_selection_audit_exclusion(
                    &mut selection_audit_exclusions,
                    &row,
                    "notAssignable",
                );
                saw_other_non_rate_limited_routing_candidate = true;
            }
        }
    }

    resolved_candidates
        .sort_by(|lhs, rhs| compare_pool_routing_candidate_scores(&lhs.score, &rhs.score));
    selection_audit_exclusions.retain(|excluded| {
        !resolved_candidates.iter().any(|candidate| {
            candidate
                .resolved_account
                .as_ref()
                .is_some_and(|account| account.account_id == excluded.account_id)
        })
    });
    let selection_audit = resolved_candidates.first().and_then(|winner| {
        let account = winner.resolved_account.as_ref()?;
        if account.routing_source != PoolRoutingSelectionSource::FreshAssignment {
            return None;
        }
        let runner_up = resolved_candidates
            .get(1)
            .and_then(|candidate| candidate.resolved_account.as_ref());
        Some(PoolRoutingSelectionAudit {
            selected_account_id: account.account_id,
            selected_account_name: account.display_name.clone(),
            eligible_candidate_count: resolved_candidates.len(),
            winner_reason_code: pool_routing_selection_winner_reason(
                &winner.score,
                resolved_candidates.get(1).map(|candidate| &candidate.score),
            )
            .to_string(),
            compared_account_id: runner_up.map(|candidate| candidate.account_id),
            compared_account_name: runner_up.map(|candidate| candidate.display_name.clone()),
            selected_score: Some(routing_selection_score_snapshot(&winner.score)),
            compared_score: resolved_candidates
                .get(1)
                .map(|candidate| routing_selection_score_snapshot(&candidate.score)),
            excluded_candidates: selection_audit_exclusions
                .iter()
                .take(POOL_ROUTING_SELECTION_AUDIT_EXCLUSION_LIMIT)
                .cloned()
                .collect(),
        })
    });
    let cache_hit_protection = routing_snapshot.cache_hit_protection();
    let eligible_candidate_count = resolved_candidates.len();
    let mut reservation_conflict_count = 0_usize;
    if let Some((sticky_account_id, terminal_reason_code)) =
        sticky_queue_reservation_conflict.as_ref()
    {
        for evaluation in &resolved_candidates {
            let Some(account) = evaluation.resolved_account.as_ref() else {
                continue;
            };
            let concurrency_limit =
                routing_snapshot.model_route_concurrency_limit(account.account_id, requested_model);
            if !pool_routing_model_reservation_is_at_capacity(
                state,
                reservation_key.expect("sticky queue conflict requires a reservation key"),
                account.account_id,
                requested_model,
                concurrency_limit,
            ) {
                continue;
            }
            reservation_conflict_count += 1;
            let reason_code = if account.account_id == *sticky_account_id {
                terminal_reason_code.as_str()
            } else if routing_snapshot
                .model_route_requires_expired_cooldown_probe(account.account_id, requested_model)
            {
                "expiredCooldownProbe"
            } else {
                "modelConcurrencyLimit"
            };
            if !selection_audit_exclusions
                .iter()
                .any(|candidate| candidate.account_id == account.account_id)
            {
                selection_audit_exclusions.push(PoolRoutingSelectionAuditExcludedCandidate {
                    account_id: account.account_id,
                    account_name: account.display_name.clone(),
                    reason_code: reason_code.to_string(),
                });
            }
        }
        return Ok(PoolAccountResolution::NoCandidate(
            no_candidate_audit_from_snapshot(
                routing_snapshot.as_ref(),
                requested_model,
                terminal_reason_code,
                candidate_count,
                eligible_candidate_count,
                reservation_conflict_count,
                &selection_audit_exclusions,
            ),
        ));
    }
    let mut resolved_candidates = resolved_candidates.into_iter();
    while let Some(evaluation) = resolved_candidates.next() {
        if let Some(account) = evaluation.resolved_account {
            if let Some(reservation_key) = reservation_key {
                let concurrency_limit = routing_snapshot
                    .model_route_concurrency_limit(account.account_id, requested_model);
                if !try_reserve_pool_routing_account_for_model(
                    state,
                    reservation_key,
                    &account,
                    requested_model,
                    concurrency_limit,
                ) {
                    reservation_conflict_count += 1;
                    let reason_code = if routing_snapshot
                        .model_route_requires_expired_cooldown_probe(
                            account.account_id,
                            requested_model,
                        ) {
                        "expiredCooldownProbe"
                    } else if account.routing_source == PoolRoutingSelectionSource::StickyReuse {
                        "stickyRouteReservationConflict"
                    } else {
                        "modelConcurrencyLimit"
                    };
                    if !selection_audit_exclusions
                        .iter()
                        .any(|candidate| candidate.account_id == account.account_id)
                    {
                        selection_audit_exclusions.push(
                            PoolRoutingSelectionAuditExcludedCandidate {
                                account_id: account.account_id,
                                account_name: account.display_name.clone(),
                                reason_code: reason_code.to_string(),
                            },
                        );
                    }
                    if cache_hit_protection.overflow_mode == CacheHitOverflowMode::Queue {
                        for remaining in resolved_candidates {
                            let Some(remaining_account) = remaining.resolved_account else {
                                continue;
                            };
                            let remaining_limit = routing_snapshot.model_route_concurrency_limit(
                                remaining_account.account_id,
                                requested_model,
                            );
                            if !pool_routing_model_reservation_is_at_capacity(
                                state,
                                reservation_key,
                                remaining_account.account_id,
                                requested_model,
                                remaining_limit,
                            ) {
                                continue;
                            }
                            reservation_conflict_count += 1;
                            let remaining_reason = if routing_snapshot
                                .model_route_requires_expired_cooldown_probe(
                                    remaining_account.account_id,
                                    requested_model,
                                ) {
                                "expiredCooldownProbe"
                            } else if remaining_account.routing_source
                                == PoolRoutingSelectionSource::StickyReuse
                            {
                                "stickyRouteReservationConflict"
                            } else {
                                "modelConcurrencyLimit"
                            };
                            if !selection_audit_exclusions.iter().any(|candidate| {
                                candidate.account_id == remaining_account.account_id
                            }) {
                                selection_audit_exclusions.push(
                                    PoolRoutingSelectionAuditExcludedCandidate {
                                        account_id: remaining_account.account_id,
                                        account_name: remaining_account.display_name,
                                        reason_code: remaining_reason.to_string(),
                                    },
                                );
                            }
                        }
                        return Ok(PoolAccountResolution::NoCandidate(
                            no_candidate_audit_from_snapshot(
                                routing_snapshot.as_ref(),
                                requested_model,
                                reason_code,
                                candidate_count,
                                eligible_candidate_count,
                                reservation_conflict_count,
                                &selection_audit_exclusions,
                            ),
                        ));
                    }
                    saw_model_concurrency_limited_candidate = true;
                    continue;
                }
            }
            let account = account.with_sticky_affinity_generation(sticky_affinity_generation);
            let account = if account.routing_source == PoolRoutingSelectionSource::FreshAssignment {
                account.with_routing_selection_audit(
                    selection_audit.expect("resolved fresh assignment should have an audit"),
                )
            } else {
                account
            };
            return Ok(PoolAccountResolution::Resolved(account));
        }
    }

    // Reroute only falls through to an ordinary unavailable result when capacity
    // was not the limiting condition. A forced/sticky route with no legal
    // alternate must wait on the existing bounded no-account path.
    if saw_model_concurrency_limited_candidate {
        let terminal_reason_code = if selection_audit_exclusions
            .iter()
            .any(|candidate| candidate.reason_code == "expiredCooldownProbe")
        {
            "expiredCooldownProbe"
        } else {
            "modelConcurrencyLimit"
        };
        return Ok(PoolAccountResolution::NoCandidate(
            no_candidate_audit_from_snapshot(
                routing_snapshot.as_ref(),
                requested_model,
                terminal_reason_code,
                candidate_count,
                eligible_candidate_count,
                reservation_conflict_count,
                &selection_audit_exclusions,
            ),
        ));
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

    Ok(PoolAccountResolution::NoCandidate(
        no_candidate_audit_from_snapshot(
            routing_snapshot.as_ref(),
            requested_model,
            if selection_audit_exclusions.iter().any(|candidate| {
                matches!(
                    candidate.reason_code.as_str(),
                    "policyExcluded"
                        | "bindingConstraint"
                        | "modelNotAllowed"
                        | "capabilityUnsupported"
                )
            }) {
                "policyExcluded"
            } else {
                "noEligibleCandidate"
            },
            candidate_count,
            eligible_candidate_count,
            reservation_conflict_count,
            &selection_audit_exclusions,
        ),
    ))
}

async fn reserve_sticky_model_route(
    state: &AppState,
    reservation_key: Option<&str>,
    account: &PoolResolvedAccount,
    requested_model: Option<&str>,
) -> Result<bool> {
    let Some(reservation_key) = reservation_key else {
        return Ok(true);
    };
    let concurrency_limit = state.pool_routing_snapshot.current().and_then(|snapshot| {
        snapshot.model_route_concurrency_limit(account.account_id, requested_model)
    });
    Ok(try_reserve_pool_routing_account_for_model(
        state,
        reservation_key,
        account,
        requested_model,
        concurrency_limit,
    ))
}

pub(crate) fn request_capability_requirements_after_codex_imagegen_rewrite(
    endpoint: &str,
    image_intent: crate::ImageIntent,
    requested_model: Option<&str>,
    codex_imagegen_request: bool,
    rule: &EffectiveRoutingRule,
) -> RequestCapabilityRequirements {
    let hosted_image_intent = if codex_imagegen_request
        && rule.codex_imagegen_rewrite_mode != crate::CodexImagegenRewriteMode::KeepOriginal
        && image_intent == crate::ImageIntent::Yes
        && !requested_model.is_some_and(crate::is_openai_image_generation_model)
    {
        crate::ImageIntent::No
    } else {
        image_intent
    };
    let mut requirements = RequestCapabilityRequirements::from_endpoint_and_image_intent(
        endpoint,
        hosted_image_intent,
    );
    let codex_imagegen_rewrite_applies = match rule.codex_imagegen_rewrite_mode {
        crate::CodexImagegenRewriteMode::ForceAdd => true,
        crate::CodexImagegenRewriteMode::FillMissing => image_intent == crate::ImageIntent::Yes,
        crate::CodexImagegenRewriteMode::KeepOriginal
        | crate::CodexImagegenRewriteMode::ForceRemove => false,
    };
    requirements.codex_imagegen = codex_imagegen_request
        && codex_imagegen_rewrite_applies
        && matches!(endpoint, "/v1/responses" | "/v1/responses/compact");
    requirements
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing_score(account_id: i64) -> PoolRoutingCandidateScore {
        PoolRoutingCandidateScore {
            eligibility: PoolRoutingCandidateEligibility::Assignable,
            route_binding_failure_penalty: 0,
            model_route_penalty: 0,
            routing_priority_rank: 0,
            capacity_lane: PoolRoutingCandidateCapacityLane::Primary,
            dispatch_state: PoolRoutingCandidateDispatchState::ReadyOnOwnedNode,
            single_account_rotation_enabled: false,
            secondary_reset_proximity_secs: None,
            primary_reset_proximity_secs: None,
            scarcity_score: 0.0,
            effective_load: 0,
            last_selected_at: None,
            account_id,
        }
    }

    #[test]
    fn routing_selection_audit_winner_reason_matches_retry_original_precedence() {
        let mut winner = routing_score(1);
        winner.capacity_lane = PoolRoutingCandidateCapacityLane::Overflow;
        let mut runner_up = routing_score(2);
        runner_up.dispatch_state = PoolRoutingCandidateDispatchState::RetryOriginalNode;

        assert!(compare_pool_routing_candidate_scores(&winner, &runner_up).is_lt());
        assert_eq!(
            pool_routing_selection_winner_reason(&winner, Some(&runner_up)),
            "avoidsRetryOriginalNode"
        );
    }

    #[test]
    fn routing_selection_audit_winner_reason_ignores_reset_proximity_without_rotation() {
        let mut winner = routing_score(1);
        winner.secondary_reset_proximity_secs = Some(30);
        let mut runner_up = routing_score(2);
        runner_up.secondary_reset_proximity_secs = Some(10);

        assert!(compare_pool_routing_candidate_scores(&winner, &runner_up).is_lt());
        assert_eq!(
            pool_routing_selection_winner_reason(&winner, Some(&runner_up)),
            "stableAccountOrder"
        );
    }

    #[test]
    fn routing_selection_score_snapshot_preserves_the_compared_penalties() {
        let mut winner = routing_score(1);
        winner.model_route_penalty = 0;
        let mut runner_up = routing_score(2);
        runner_up.model_route_penalty = 1;

        let audit = PoolRoutingSelectionAudit {
            selected_account_id: winner.account_id,
            selected_account_name: "dzw".to_string(),
            eligible_candidate_count: 2,
            winner_reason_code: "lowerModelRoutePenalty".to_string(),
            compared_account_id: Some(runner_up.account_id),
            compared_account_name: Some("CIII".to_string()),
            selected_score: Some(routing_selection_score_snapshot(&winner)),
            compared_score: Some(routing_selection_score_snapshot(&runner_up)),
            excluded_candidates: Vec::new(),
        };
        let value = serde_json::to_value(audit).expect("serialize routing selection audit");
        assert_eq!(value["selectedScore"]["modelRoutePenalty"], 0);
        assert_eq!(value["selectedScore"]["modelRoutePenaltyCode"], "normal");
        assert_eq!(value["comparedScore"]["modelRoutePenalty"], 1);
        assert_eq!(value["comparedScore"]["modelRoutePenaltyCode"], "demoted");
    }

    #[test]
    fn no_candidate_audit_keeps_full_reason_counts_with_bounded_details() {
        let exclusions = (1..=15)
            .map(|account_id| PoolRoutingSelectionAuditExcludedCandidate {
                account_id,
                account_name: format!("Account {account_id}"),
                reason_code: "policyExcluded".to_string(),
            })
            .collect::<Vec<_>>();

        let audit = no_candidate_audit("policyExcluded", 15, 0, 0, &exclusions);

        assert_eq!(audit.excluded_reason_counts["policyExcluded"], 15);
        assert_eq!(
            audit.candidates.len(),
            POOL_ROUTING_SELECTION_AUDIT_EXCLUSION_LIMIT
        );
    }

    fn effective_rule(
        codex_imagegen_rewrite_mode: crate::CodexImagegenRewriteMode,
    ) -> EffectiveRoutingRule {
        EffectiveRoutingRule {
            allow_cut_out: true,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: crate::ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode,
            request_compression_algorithm: RequestCompressionAlgorithm::Identity,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: Vec::new(),
            available_models_mode: AvailableModelsMode::Allowlist,
            available_models_defined: false,
            tag_available_models: None,
            status_change_reasons: default_status_change_reasons(),
            status_change_reason_field_sources: default_status_change_reason_field_sources("root"),
            system_denied_models: Vec::new(),
            source_tag_ids: Vec::new(),
            source_tag_names: Vec::new(),
            field_sources: EffectiveRoutingRuleFieldSources {
                allow_cut_out: "root".to_string(),
                allow_cut_in: "root".to_string(),
                priority_tier: "root".to_string(),
                fast_mode_rewrite_mode: "root".to_string(),
                image_tool_rewrite_mode: "root".to_string(),
                codex_imagegen_rewrite_mode: "root".to_string(),
                request_compression_algorithm: "root".to_string(),
                concurrency_limit: "root".to_string(),
                upstream_429_retry: "root".to_string(),
                available_models: "root".to_string(),
                available_models_mode: "root".to_string(),
                system_denied_models: "root".to_string(),
            },
            timeouts: RoutingTimeoutSettings::default(),
            timeout_field_sources: RoutingTimeoutFieldSources {
                responses_first_byte_timeout_secs: "root".to_string(),
                compact_first_byte_timeout_secs: "root".to_string(),
                image_first_byte_timeout_secs: "root".to_string(),
                responses_stream_timeout_secs: "root".to_string(),
                compact_stream_timeout_secs: "root".to_string(),
            },
        }
    }

    #[test]
    fn codex_non_default_rewrite_drops_hosted_image_tool_requirement() {
        let keep_original = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-5.6-codex"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::KeepOriginal),
        );
        let force_add = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-5.6-codex"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::ForceAdd),
        );

        assert!(keep_original.response_image_tool);
        assert!(!force_add.response_image_tool);
        assert!(force_add.codex_imagegen);
        assert!(force_add.response_endpoint);
        assert!(!account_accepts_request_capabilities(
            force_add,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Unsupported,
            CapabilitySupport::Unknown,
        ));
        assert!(account_accepts_request_capabilities(
            force_add,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            CapabilitySupport::Supported,
            effective_capability_support(
                CapabilitySupport::Unsupported,
                Some(CapabilitySupport::Supported),
            ),
            CapabilitySupport::Unknown,
        ));

        let image_model = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-image-1"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::ForceAdd),
        );
        assert!(image_model.response_image_tool);
        assert!(image_model.codex_imagegen);

        let force_remove = request_capability_requirements_after_codex_imagegen_rewrite(
            "/v1/responses",
            crate::ImageIntent::Yes,
            Some("gpt-5.6-codex"),
            true,
            &effective_rule(crate::CodexImagegenRewriteMode::ForceRemove),
        );
        assert!(!force_remove.codex_imagegen);

        let fill_missing_without_image_intent =
            request_capability_requirements_after_codex_imagegen_rewrite(
                "/v1/responses",
                crate::ImageIntent::Unknown,
                Some("gpt-5.6-codex"),
                true,
                &effective_rule(crate::CodexImagegenRewriteMode::FillMissing),
            );
        assert!(!fill_missing_without_image_intent.codex_imagegen);
    }
}
