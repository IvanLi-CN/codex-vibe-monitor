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
    )
    .await
}

pub(crate) async fn resolve_pool_account_for_request_with_route_requirement(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    required_upstream_route_key: Option<&str>,
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
    let mut sticky_route_group_proxy_blocked_message = None;
    let mut group_proxy_blocked_messages = Vec::new();
    let mut node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;

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

    if let Some(route) = sticky_route.as_ref() {
        if !tried.contains(&route.account_id)
            && let Some(row) = load_upstream_account_row(&state.pool, route.account_id).await?
        {
            tried.insert(route.account_id);
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
            let sticky_route_is_excluded_by_route_key = sticky_route_key
                .as_deref()
                .is_some_and(|route_key| excluded_upstream_route_keys.contains(route_key));
            if !sticky_route_matches_required {
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
                        let prepared_account = prepare_pool_account_with_node_shunt_refresh(
                            state,
                            &row,
                            sticky_source_rule
                                .as_ref()
                                .expect("sticky source rule should be loaded"),
                            &group_metadata,
                            &mut node_shunt_assignments,
                        )
                        .await;
                        let account = match prepared_account {
                            Ok(account) => account,
                            Err(err)
                                if is_group_node_shunt_unassigned_message(&err.to_string()) =>
                            {
                                sticky_route_group_proxy_blocked_message = Some(err.to_string());
                                group_proxy_blocked_messages.push(err.to_string());
                                None
                            }
                            Err(err) => return Err(err),
                        };
                        if let Some(account) = account {
                            let mut account = account;
                            account.routing_source = PoolRoutingSelectionSource::StickyReuse;
                            if !excluded_upstream_route_keys.contains(&account.upstream_route_key())
                            {
                                return Ok(PoolAccountResolution::Resolved(account));
                            }
                            sticky_route_excluded_by_route_key = true;
                            sticky_route_was_excluded = true;
                            if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now)
                            {
                                saw_degraded_candidate = true;
                            } else {
                                saw_excluded_route_candidate = true;
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
                            group_proxy_blocked_messages.push(message);
                        }
                    }
                }
                if !sticky_route_was_excluded {
                    if sticky_route_group_proxy_blocked_message.is_none() {
                        if is_account_degraded_for_routing(&row, sticky_snapshot_exhausted, now) {
                            saw_degraded_candidate = true;
                        } else {
                            saw_other_non_rate_limited_routing_candidate = true;
                        }
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
                // Active accounts without usable credentials are not real
                // routing candidates and should not mask an all-429 pool.
                saw_non_routing_candidate = true;
            }
        }
        if sticky_source_rule
            .as_ref()
            .is_some_and(|rule| !rule.allow_cut_out)
            && sticky_route_still_reusable
            && !sticky_route_excluded_by_route_key
        {
            if let Some(message) = sticky_route_group_proxy_blocked_message {
                return Ok(PoolAccountResolution::BlockedByPolicy(message));
            }
            return Ok(PoolAccountResolution::BlockedByPolicy(
                "sticky conversation cannot cut out of the current account because a tag rule forbids it"
                    .to_string(),
            ));
        }
    }

    let mut candidates = load_account_routing_candidates(&state.pool, &tried).await?;
    for candidate in &mut candidates {
        candidate.in_flight_reservations = pool_routing_reservation_count(state, candidate.id);
    }
    let candidate_effective_rules = load_effective_routing_rules_for_accounts(
        &state.pool,
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
    )
    .await?;
    candidates.sort_by(compare_routing_candidates);
    let mut primary_candidates = [Vec::new(), Vec::new(), Vec::new()];
    let mut overflow_candidates = [Vec::new(), Vec::new(), Vec::new()];
    for candidate in candidates {
        let priority_index = usize::from(routing_priority_rank(
            candidate_effective_rules.get(&candidate.id),
        ));
        if candidate.effective_load() < candidate.capacity_profile().hard_cap {
            primary_candidates[priority_index].push(candidate);
        } else {
            overflow_candidates[priority_index].push(candidate);
        }
    }
    let mut candidate_passes = Vec::new();
    for priority_index in 0..=2 {
        if primary_candidates[priority_index].is_empty() {
            if !overflow_candidates[priority_index].is_empty() {
                candidate_passes.push(std::mem::take(&mut overflow_candidates[priority_index]));
            }
            continue;
        }
        candidate_passes.push(std::mem::take(&mut primary_candidates[priority_index]));
        if !overflow_candidates[priority_index].is_empty() {
            candidate_passes.push(std::mem::take(&mut overflow_candidates[priority_index]));
        }
    }
    for pass_candidates in candidate_passes {
        for candidate in pass_candidates {
            let Some(row) = load_upstream_account_row(&state.pool, candidate.id).await? else {
                continue;
            };
            let snapshot_exhausted = routing_candidate_snapshot_is_exhausted(&candidate);
            let candidate_route_key = resolve_pool_account_upstream_base_url(
                &row,
                &state.config.openai_upstream_base_url,
            )
            .ok()
            .map(|url| canonical_pool_upstream_route_key(&url));
            let candidate_route_matches_required =
                required_upstream_route_key.is_none_or(|required| {
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
            if !is_account_selectable_for_fresh_assignment(&row, snapshot_exhausted, now) {
                if candidate_route_is_excluded_by_route_key
                    && (is_account_rate_limited_for_routing(&row, snapshot_exhausted)
                        || is_account_degraded_for_routing(&row, snapshot_exhausted, now)
                        || is_routing_eligible_account(&row))
                {
                    saw_excluded_route_candidate = true;
                } else if is_account_rate_limited_for_routing(&row, snapshot_exhausted) {
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
            )
            .await?
            {
                if candidate_route_is_excluded_by_route_key {
                    saw_excluded_route_candidate = true;
                } else {
                    saw_other_non_rate_limited_routing_candidate = true;
                }
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
                    if candidate_route_is_excluded_by_route_key {
                        saw_excluded_route_candidate = true;
                    } else {
                        group_proxy_blocked_messages.push(message);
                    }
                    continue;
                }
            };
            let prepared_account = prepare_pool_account_with_node_shunt_refresh(
                state,
                &row,
                effective_rule,
                &group_metadata,
                &mut node_shunt_assignments,
            )
            .await;
            let account = match prepared_account {
                Ok(account) => account,
                Err(err) if is_group_node_shunt_unassigned_message(&err.to_string()) => {
                    if candidate_route_is_excluded_by_route_key {
                        saw_excluded_route_candidate = true;
                    } else {
                        group_proxy_blocked_messages.push(err.to_string());
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            if let Some(account) = account {
                if excluded_upstream_route_keys.contains(&account.upstream_route_key()) {
                    saw_excluded_route_candidate = true;
                    continue;
                }
                return Ok(PoolAccountResolution::Resolved(account));
            }
            saw_other_non_rate_limited_routing_candidate = true;
        }
    }

    // Surface concrete group-proxy misconfiguration before generic pool exhaustion
    // when every transferable fresh candidate was filtered for that reason,
    // even if the rest of the pool is already rate-limited or degraded.
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
