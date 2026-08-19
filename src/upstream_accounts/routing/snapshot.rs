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
    route_binding_failure_penalties: HashMap<String, i64>,
    transport_decode_sticky_escape_states: HashMap<i64, TransportDecodeStickyEscapeState>,
    group_metadata: HashMap<String, UpstreamAccountGroupMetadata>,
    sticky_routes: HashMap<String, PoolStickyRouteRow>,
    sticky_model_routes: HashMap<(String, String), PoolStickyRouteRow>,
    sticky_generations: HashMap<String, i64>,
    sticky_model_generations: HashMap<(String, String), i64>,
    cache_hit_protection: CacheHitProtectionSettings,
}

#[derive(Debug, Clone)]
struct PoolRoutingModelRouteSnapshot {
    state: String,
    cooldown_until: Option<String>,
    concurrency_limit: Option<i64>,
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
                    .map(|route| (*account_id, route.penalty_at(Utc::now())))
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
        if route.probe_required_at(Utc::now()) {
            return Some(1);
        }
        self.cache_hit_protection
            .enabled
            .then_some(route.concurrency_limit)
            .flatten()
            .map(|limit| limit.max(1))
    }

    pub(crate) fn model_route_requires_expired_cooldown_probe(
        &self,
        account_id: i64,
        model: Option<&str>,
    ) -> bool {
        let Some(model) = normalized_model_key(model) else {
            return false;
        };
        self.model_routes
            .get(&(account_id, model))
            .is_some_and(|route| route.probe_required_at(Utc::now()))
    }

    pub(crate) fn route_binding_failure_penalties(&self) -> &HashMap<String, i64> {
        &self.route_binding_failure_penalties
    }

    pub(crate) fn transport_decode_sticky_escape_states(
        &self,
        account_ids: &[i64],
    ) -> HashMap<i64, TransportDecodeStickyEscapeState> {
        let now = Utc::now();
        account_ids
            .iter()
            .filter_map(|account_id| {
                self.transport_decode_sticky_escape_states
                    .get(account_id)
                    .copied()
                    .filter(|state| now < state.until)
                    .map(|state| (*account_id, state))
            })
            .collect()
    }

    pub(crate) fn group_metadata(&self, group_name: Option<&str>) -> UpstreamAccountGroupMetadata {
        group_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .and_then(|name| self.group_metadata.get(name))
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn sticky_route_with_model_generation(
        &self,
        sticky_key: &str,
        requested_model: Option<&str>,
    ) -> (Option<PoolStickyRouteRow>, i64) {
        let Some(model_key) = normalize_sticky_model_key(requested_model) else {
            return (
                self.sticky_routes.get(sticky_key).cloned(),
                self.sticky_generations
                    .get(sticky_key)
                    .copied()
                    .unwrap_or_default(),
            );
        };
        let route = self
            .sticky_model_routes
            .get(&(sticky_key.to_string(), model_key.clone()))
            .cloned()
            .or_else(|| self.sticky_routes.get(sticky_key).cloned());
        let epoch = self
            .sticky_generations
            .get(sticky_key)
            .copied()
            .unwrap_or_default();
        let generation = self
            .sticky_model_generations
            .get(&(sticky_key.to_string(), model_key))
            .copied()
            .unwrap_or_default();
        (route, pack_sticky_affinity_token(epoch, generation))
    }

    pub(crate) fn earliest_model_route_cooldown_expiry_for_accounts(
        &self,
        model: Option<&str>,
        account_ids: &[i64],
    ) -> Option<String> {
        let model = normalized_model_key(model)?;
        let now = Utc::now();
        account_ids
            .iter()
            .filter_map(|account_id| self.model_routes.get(&(*account_id, model.clone())))
            .filter_map(|route| route.cooldown_until.as_deref())
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
        .map(str::to_string)
}

impl PoolRoutingModelRouteSnapshot {
    fn cooldown_expired(&self, now: DateTime<Utc>) -> bool {
        self.cooldown_until
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_some_and(|until| until <= now)
    }

    fn penalty_at(&self, now: DateTime<Utc>) -> ModelRoutePenalty {
        if self.state == MODEL_ROUTE_STATE_COOLING_DOWN && !self.cooldown_expired(now) {
            ModelRoutePenalty::Excluded
        } else if self.state == MODEL_ROUTE_STATE_DEGRADED
            || self.state == MODEL_ROUTE_STATE_COOLING_DOWN
        {
            ModelRoutePenalty::Demoted
        } else {
            ModelRoutePenalty::Normal
        }
    }

    fn probe_required_at(&self, now: DateTime<Utc>) -> bool {
        self.state == MODEL_ROUTE_STATE_COOLING_DOWN && self.cooldown_expired(now)
    }
}

#[derive(Debug)]
pub(crate) struct PoolRoutingSnapshotStore {
    snapshot: std::sync::RwLock<Option<Arc<PoolRoutingSnapshot>>>,
    refresh_tx: tokio::sync::watch::Sender<u64>,
    refresh_state: std::sync::Mutex<PoolRoutingSnapshotRefreshState>,
}

#[derive(Debug, Default)]
struct PoolRoutingSnapshotRefreshState {
    generation: u64,
    pending: bool,
    wake_waiters: bool,
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
            refresh_state: std::sync::Mutex::new(PoolRoutingSnapshotRefreshState::default()),
        }
    }

    pub(crate) fn current(&self) -> Option<Arc<PoolRoutingSnapshot>> {
        self.snapshot
            .read()
            .expect("pool routing snapshot lock poisoned")
            .clone()
    }

    pub(crate) fn request_refresh(&self) {
        self.request_refresh_inner(false);
    }

    pub(crate) fn request_refresh_and_wake_waiters(&self) {
        self.request_refresh_inner(true);
    }

    fn request_refresh_inner(&self, wake_waiters: bool) {
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
        refresh_state.pending = true;
        refresh_state.wake_waiters |= wake_waiters;
        refresh_state.generation = refresh_state.generation.wrapping_add(1);
        drop(refresh_state);
        self.refresh_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    pub(crate) fn subscribe_refresh(&self) -> tokio::sync::watch::Receiver<u64> {
        self.refresh_tx.subscribe()
    }

    pub(crate) fn complete_refresh(
        &self,
        refresh_generation: u64,
        snapshot: PoolRoutingSnapshot,
        publish_availability: impl FnOnce(),
    ) -> bool {
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
        if refresh_state.generation != refresh_generation {
            return false;
        }
        *self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned") = Some(Arc::new(snapshot));
        let wake_waiters = std::mem::take(&mut refresh_state.wake_waiters);
        refresh_state.pending = false;
        if wake_waiters {
            publish_availability();
        }
        true
    }

    pub(crate) fn invalidate(&self) {
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
        *self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned") = None;
        refresh_state.pending = false;
        refresh_state.wake_waiters = false;
        refresh_state.generation = refresh_state.generation.wrapping_add(1);
    }

    pub(crate) fn refresh_pending(&self) -> bool {
        self.refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned")
            .pending
    }

    pub(crate) fn refresh_generation(&self) -> u64 {
        self.refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned")
            .generation
    }

    pub(crate) fn publish_availability_if_ready(&self, publish_availability: impl FnOnce()) {
        if !self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned")
            .pending
        {
            publish_availability();
        }
    }
}

pub(crate) async fn refresh_pool_routing_snapshot(state: &AppState) -> Result<()> {
    let refresh_generation = state.pool_routing_snapshot.refresh_generation();
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
    let route_binding_failure_penalties =
        load_recent_route_binding_failure_penalties(&state.pool).await?;
    let transport_decode_sticky_escape_states =
        load_transport_decode_sticky_escape_states(&state.pool, &account_ids).await?;
    let node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    let settings = load_pool_routing_settings(&state.pool).await?;
    let model_rows = sqlx::query_as::<_, (i64, String, String, Option<String>, Option<i64>)>(
        "SELECT routes.account_id, routes.model, routes.state, routes.cooldown_until, routes.cache_concurrency_limit \
         FROM pool_upstream_account_model_routes routes \
         INNER JOIN pool_upstream_accounts accounts ON accounts.id = routes.account_id \
         WHERE accounts.kind = ?1 AND COALESCE(accounts.deleted_at, '') = '' AND routes.last_seen_at >= ?2",
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .bind((Utc::now() - ChronoDuration::days(MODEL_ROUTE_RETENTION_DAYS)).to_rfc3339())
    .fetch_all(&state.pool)
    .await?;
    let model_routes = model_rows
        .into_iter()
        .map(
            |(account_id, model, state, cooldown_until, concurrency_limit)| {
                (
                    (account_id, model.trim().to_string()),
                    PoolRoutingModelRouteSnapshot {
                        state,
                        cooldown_until,
                        concurrency_limit,
                    },
                )
            },
        )
        .collect();
    let group_metadata = sqlx::query_as::<_, (String, String, Option<String>, i64, i64, i64, i64, i64)>(
        "SELECT group_name, note, bound_proxy_keys_json, node_shunt_enabled, single_account_rotation_enabled, upstream_429_retry_enabled, upstream_429_max_retries, concurrency_limit FROM pool_upstream_account_group_notes",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(group_name, note, bound_proxy_keys_json, node_shunt_enabled, single_account_rotation_enabled, upstream_429_retry_enabled, upstream_429_max_retries, concurrency_limit)| {
        let upstream_429_retry_enabled = decode_group_upstream_429_retry_enabled(upstream_429_retry_enabled);
        (group_name, UpstreamAccountGroupMetadata {
            note: normalize_optional_text(Some(note)),
            bound_proxy_keys: decode_group_bound_proxy_keys_json(bound_proxy_keys_json.as_deref()),
            node_shunt_enabled: decode_group_node_shunt_enabled(node_shunt_enabled),
            single_account_rotation_enabled: decode_group_single_account_rotation_enabled(single_account_rotation_enabled),
            upstream_429_retry_enabled,
            upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(upstream_429_retry_enabled, decode_group_upstream_429_max_retries(upstream_429_max_retries)),
            concurrency_limit,
        })
    })
    .collect();
    let sticky_routes = sqlx::query_as::<_, PoolStickyRouteRow>(
        "SELECT sticky_key, account_id, created_at, updated_at, last_seen_at FROM pool_sticky_routes",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|route| (route.sticky_key.clone(), route))
    .collect();
    let sticky_model_routes = sqlx::query_as::<_, PoolStickyModelRouteRow>(
        "SELECT sticky_key, model_key, account_id, created_at, updated_at, last_seen_at FROM pool_sticky_model_routes",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|route| {
        let key = (route.sticky_key.clone(), route.model_key);
        (key, PoolStickyRouteRow { sticky_key: route.sticky_key, account_id: route.account_id, created_at: route.created_at, updated_at: route.updated_at, last_seen_at: route.last_seen_at })
    })
    .collect();
    let sticky_generations = sqlx::query_as::<_, (String, i64)>(
        "SELECT sticky_key, generation FROM pool_sticky_route_generations",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .collect();
    let sticky_model_generations = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT sticky_key, model_key, generation FROM pool_sticky_model_route_generations",
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(sticky_key, model_key, generation)| ((sticky_key, model_key), generation))
    .collect();
    state.pool_routing_snapshot.complete_refresh(
        refresh_generation,
        PoolRoutingSnapshot {
            candidate_order: candidates.iter().map(|candidate| candidate.id).collect(),
            candidates: candidates
                .into_iter()
                .map(|candidate| (candidate.id, candidate))
                .collect(),
            accounts,
            effective_rules,
            node_shunt_assignments,
            model_routes,
            route_binding_failure_penalties,
            transport_decode_sticky_escape_states,
            group_metadata,
            sticky_routes,
            sticky_model_routes,
            sticky_generations,
            sticky_model_generations,
            cache_hit_protection: resolve_cache_hit_protection_settings(&settings),
        },
        || state.pool_routing_availability.publish(),
    );
    Ok(())
}
