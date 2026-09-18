async fn collect_node_shunt_candidates(
    state: &AppState,
    group_metadata_map: &HashMap<String, UpstreamAccountGroupMetadata>,
    rows_by_id: &HashMap<i64, UpstreamAccountRow>,
    now: DateTime<Utc>,
    assignments: &mut UpstreamAccountNodeShuntAssignments,
    reservation_snapshot: &PoolRoutingReservationSnapshot,
) -> Result<(
    HashMap<String, Vec<AccountRoutingCandidateRow>>,
    HashMap<i64, EffectiveRoutingRule>,
)> {
    let mut candidates = load_account_routing_candidates(&state.pool, &HashSet::new()).await?;
    for candidate in &mut candidates {
        candidate.in_flight_reservations = reservation_snapshot.count_for_account(candidate.id);
    }
    let candidate_effective_rules = load_effective_routing_rules_for_accounts(
        &state.pool,
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
    )
    .await?;
    let mut group_candidates = HashMap::<String, Vec<AccountRoutingCandidateRow>>::new();
    for candidate in candidates {
        let Some(row) = rows_by_id.get(&candidate.id) else {
            continue;
        };
        let Some(group_name) = normalize_optional_text(row.group_name.clone()) else {
            continue;
        };
        if !group_metadata_map
            .get(&group_name)
            .is_some_and(|metadata| metadata.node_shunt_enabled)
            || !account_is_node_shunt_slot_eligible(
                row,
                routing_candidate_snapshot_is_exhausted(&candidate),
                now,
            )
        {
            continue;
        }
        assignments.eligible_account_ids.insert(row.id);
        group_candidates
            .entry(group_name)
            .or_default()
            .push(candidate);
    }
    Ok((group_candidates, candidate_effective_rules))
}

fn sort_node_shunt_candidates(
    group_candidates: &HashMap<String, Vec<AccountRoutingCandidateRow>>,
    effective_rules: &HashMap<i64, EffectiveRoutingRule>,
) -> (
    Vec<(String, AccountRoutingCandidateRow)>,
    Vec<(String, AccountRoutingCandidateRow)>,
) {
    let mut reserved = group_candidates
        .iter()
        .flat_map(|(group, candidates)| {
            candidates
                .iter()
                .filter(|candidate| candidate.in_flight_reservations > 0)
                .cloned()
                .map(|candidate| (group.clone(), candidate))
        })
        .collect::<Vec<_>>();
    reserved.sort_by(|(left_group, left), (right_group, right)| {
        routing_priority_rank(effective_rules.get(&left.id))
            .cmp(&routing_priority_rank(effective_rules.get(&right.id)))
            .then_with(|| compare_node_shunt_reserved_candidates(left, right))
            .then_with(|| left_group.cmp(right_group))
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut fresh = group_candidates
        .iter()
        .flat_map(|(group, candidates)| {
            candidates
                .iter()
                .cloned()
                .map(|candidate| (group.clone(), candidate))
        })
        .collect::<Vec<_>>();
    fresh.sort_by(|(left_group, left), (right_group, right)| {
        routing_priority_rank(effective_rules.get(&left.id))
            .cmp(&routing_priority_rank(effective_rules.get(&right.id)))
            .then_with(|| compare_routing_candidates(left, right))
            .then_with(|| left_group.cmp(right_group))
            .then_with(|| left.id.cmp(&right.id))
    });
    (reserved, fresh)
}
