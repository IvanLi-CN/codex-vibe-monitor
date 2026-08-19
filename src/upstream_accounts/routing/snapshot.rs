use super::*;

pub(crate) const POOL_ROUTING_SNAPSHOT_RECONCILE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingSnapshot {
    candidates: HashMap<i64, AccountRoutingCandidateRow>,
    candidate_order: Vec<i64>,
    accounts: HashMap<i64, UpstreamAccountRow>,
    effective_rules: HashMap<i64, EffectiveRoutingRule>,
    node_shunt_assignments: UpstreamAccountNodeShuntAssignments,
    model_routes: HashMap<(i64, String), PoolRoutingModelRouteSnapshot>,
    cache_hit_protection: CacheHitProtectionSettings,
}

#[derive(Debug, Clone)]
struct PoolRoutingModelRouteSnapshot {
    penalty: ModelRoutePenalty,
    cooldown_until: Option<String>,
    concurrency_limit: Option<i64>,
    probe_required: bool,
}

impl PoolRoutingSnapshot {
    pub(crate) fn candidate(&self, account_id: i64) -> Option<&AccountRoutingCandidateRow> {
        self.candidates.get(&account_id)
    }

    pub(crate) fn candidates(
        &self,
        excluded_ids: &HashSet<i64>,
    ) -> Vec<AccountRoutingCandidateRow> {
        self.candidate_order
            .iter()
            .filter(|account_id| !excluded_ids.contains(account_id))
            .filter_map(|account_id| self.candidates.get(account_id).cloned())
            .collect()
    }

    pub(crate) fn account(&self, account_id: i64) -> Option<&UpstreamAccountRow> {
        self.accounts.get(&account_id)
    }

    pub(crate) fn effective_rule(&self, account_id: i64) -> Option<&EffectiveRoutingRule> {
        self.effective_rules.get(&account_id)
    }

    pub(crate) fn effective_rules_for(
        &self,
        account_ids: &[i64],
    ) -> HashMap<i64, EffectiveRoutingRule> {
        account_ids
            .iter()
            .filter_map(|account_id| {
                self.effective_rules
                    .get(account_id)
                    .cloned()
                    .map(|rule| (*account_id, rule))
            })
            .collect()
    }

    pub(crate) fn node_shunt_assignments(&self) -> UpstreamAccountNodeShuntAssignments {
        self.node_shunt_assignments.clone()
    }

    pub(crate) fn model_route_penalties(
        &self,
        account_ids: &[i64],
        model: Option<&str>,
    ) -> HashMap<i64, ModelRoutePenalty> {
        let Some(model) = normalized_model_key(model) else {
            return HashMap::new();
        };
        account_ids
            .iter()
            .filter_map(|account_id| {
                self.model_routes
                    .get(&(*account_id, model.clone()))
                    .map(|route| (*account_id, route.penalty))
            })
            .collect()
    }

    pub(crate) fn model_route_concurrency_limit(
        &self,
        account_id: i64,
        model: Option<&str>,
    ) -> Option<i64> {
        let model = normalized_model_key(model)?;
        let route = self.model_routes.get(&(account_id, model))?;
        if route.probe_required {
            return Some(1);
        }
        self.cache_hit_protection
            .enabled
            .then_some(route.concurrency_limit)
            .flatten()
            .map(|limit| limit.max(1))
    }

    pub(crate) fn earliest_model_route_cooldown_expiry(
        &self,
        model: Option<&str>,
    ) -> Option<String> {
        let model = normalized_model_key(model)?;
        let now = Utc::now();
        self.model_routes
            .iter()
            .filter(|((_, route_model), _)| route_model == &model)
            .filter_map(|(_, route)| route.cooldown_until.as_deref())
            .filter_map(parse_to_utc_datetime)
            .filter(|until| *until > now)
            .min()
            .map(|until| until.to_rfc3339())
    }

    pub(crate) fn cache_hit_protection(&self) -> &CacheHitProtectionSettings {
        &self.cache_hit_protection
    }
}

fn normalized_model_key(model: Option<&str>) -> Option<String> {
    model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|model| model.to_ascii_lowercase())
}

#[derive(Debug)]
pub(crate) struct PoolRoutingSnapshotStore {
    snapshot: std::sync::RwLock<Option<Arc<PoolRoutingSnapshot>>>,
    refresh_tx: tokio::sync::watch::Sender<u64>,
}

impl Default for PoolRoutingSnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PoolRoutingSnapshotStore {
    pub(crate) fn new() -> Self {
        let (refresh_tx, _refresh_rx) = tokio::sync::watch::channel(0);
        Self {
            snapshot: std::sync::RwLock::new(None),
            refresh_tx,
        }
    }

    pub(crate) fn current(&self) -> Option<Arc<PoolRoutingSnapshot>> {
        self.snapshot
            .read()
            .expect("pool routing snapshot lock poisoned")
            .clone()
    }

    pub(crate) fn request_refresh(&self) {
        self.refresh_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    pub(crate) fn subscribe_refresh(&self) -> tokio::sync::watch::Receiver<u64> {
        self.refresh_tx.subscribe()
    }

    pub(crate) fn replace(&self, snapshot: PoolRoutingSnapshot) {
        *self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned") = Some(Arc::new(snapshot));
    }
}

pub(crate) async fn refresh_pool_routing_snapshot(state: &AppState) -> Result<()> {
    let candidates = load_account_routing_candidates(&state.pool, &HashSet::new()).await?;
    let account_ids = candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let mut accounts = HashMap::with_capacity(account_ids.len());
    for account_id in &account_ids {
        if let Some(account) = load_upstream_account_row(&state.pool, *account_id).await? {
            accounts.insert(*account_id, account);
        }
    }
    let effective_rules =
        load_effective_routing_rules_for_accounts(&state.pool, &account_ids).await?;
    let node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    let settings = load_pool_routing_settings(&state.pool).await?;
    let model_rows = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>)>(
        "SELECT account_id, model, state, cooldown_until, cache_concurrency_limit FROM pool_upstream_account_model_routes",
    )
    .fetch_all(&state.pool)
    .await?;
    let now = Utc::now();
    let model_routes = model_rows
        .into_iter()
        .map(
            |(account_id, model, state, cooldown_until, concurrency_limit)| {
                let probe_required = state == MODEL_ROUTE_STATE_COOLING_DOWN
                    && cooldown_until
                        .as_deref()
                        .and_then(parse_to_utc_datetime)
                        .is_some_and(|until| until <= now);
                let penalty = if state == MODEL_ROUTE_STATE_COOLING_DOWN && !probe_required {
                    ModelRoutePenalty::Excluded
                } else if state == MODEL_ROUTE_STATE_DEGRADED
                    || state == MODEL_ROUTE_STATE_COOLING_DOWN
                {
                    ModelRoutePenalty::Demoted
                } else {
                    ModelRoutePenalty::Normal
                };
                (
                    (account_id, model.to_ascii_lowercase()),
                    PoolRoutingModelRouteSnapshot {
                        penalty,
                        cooldown_until,
                        concurrency_limit,
                        probe_required,
                    },
                )
            },
        )
        .collect();
    state.pool_routing_snapshot.replace(PoolRoutingSnapshot {
        candidate_order: candidates.iter().map(|candidate| candidate.id).collect(),
        candidates: candidates
            .into_iter()
            .map(|candidate| (candidate.id, candidate))
            .collect(),
        accounts,
        effective_rules,
        node_shunt_assignments,
        model_routes,
        cache_hit_protection: resolve_cache_hit_protection_settings(&settings),
    });
    state.pool_routing_availability.publish();
    Ok(())
}
