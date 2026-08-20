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
    refresh_epoch: std::sync::atomic::AtomicU64,
    deferred_availability_wake: std::sync::atomic::AtomicBool,
    refresh_state: std::sync::Mutex<PoolRoutingSnapshotRefreshState>,
}

#[derive(Debug, Default)]
struct PoolRoutingSnapshotRefreshState {
    pending: bool,
    wake_waiters: bool,
}

const REFRESH_PENDING_BIT: u64 = 1 << 63;
const REFRESH_PUBLISHING_BIT: u64 = 1 << 62;
const REFRESH_RECONCILING_BIT: u64 = 1 << 61;
// A stale normal build gets one immediate successor. Further mutations still
// fence stale snapshots, then wait for the low-frequency reconciler rather
// than rebuilding forever.
const REFRESH_COALESCED_SUCCESSOR_BIT: u64 = 1 << 60;
const REFRESH_GENERATION_MASK: u64 = !(REFRESH_PENDING_BIT
    | REFRESH_PUBLISHING_BIT
    | REFRESH_RECONCILING_BIT
    | REFRESH_COALESCED_SUCCESSOR_BIT);

struct PoolRoutingSnapshotRefreshLease<'a> {
    store: &'a PoolRoutingSnapshotStore,
    generation: Option<u64>,
}

impl<'a> PoolRoutingSnapshotRefreshLease<'a> {
    fn new(store: &'a PoolRoutingSnapshotStore, generation: u64) -> Self {
        Self {
            store,
            generation: Some(generation),
        }
    }

    fn complete(&mut self, snapshot: PoolRoutingSnapshot, publish_availability: impl Fn()) -> bool {
        let Some(generation) = self.generation.take() else {
            return false;
        };
        self.store
            .complete_refresh(generation, snapshot, publish_availability)
    }
}

impl Drop for PoolRoutingSnapshotRefreshLease<'_> {
    fn drop(&mut self) {
        if let Some(generation) = self.generation {
            self.store.abandon_refresh(Some(generation));
        }
    }
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
            refresh_epoch: std::sync::atomic::AtomicU64::new(0),
            deferred_availability_wake: std::sync::atomic::AtomicBool::new(false),
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
        // Atomically fence an older reconciler and make the pending state
        // visible before waiting for refresh_state. This blocks stale capacity
        // wakeups while a mutation is queued behind a completing refresh.
        let queued_refresh = self.advance_refresh_epoch();
        // The committed mutation is now newer than the visible snapshot. Do
        // not select from that stale view while an event refresh catches up.
        *self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned") = None;
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
        refresh_state.pending = true;
        refresh_state.wake_waiters |= wake_waiters;
        drop(refresh_state);
        if queued_refresh {
            self.refresh_tx.send_modify(|generation| {
                *generation = generation.wrapping_add(1);
            });
        }
    }

    pub(crate) fn subscribe_refresh(&self) -> tokio::sync::watch::Receiver<u64> {
        self.refresh_tx.subscribe()
    }

    pub(crate) fn begin_refresh(&self) -> Option<u64> {
        loop {
            let refresh_epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if refresh_epoch & REFRESH_PUBLISHING_BIT != 0 {
                std::hint::spin_loop();
                continue;
            }
            if refresh_epoch & REFRESH_RECONCILING_BIT != 0 {
                return None;
            }
            let refresh_generation = refresh_epoch & REFRESH_GENERATION_MASK;
            let successor = refresh_epoch & REFRESH_COALESCED_SUCCESSOR_BIT;
            if self
                .refresh_epoch
                .compare_exchange(
                    refresh_epoch,
                    refresh_generation | REFRESH_RECONCILING_BIT | successor,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                return Some(refresh_generation);
            }
        }
    }

    pub(crate) fn complete_refresh(
        &self,
        refresh_generation: u64,
        snapshot: PoolRoutingSnapshot,
        publish_availability: impl Fn(),
    ) -> bool {
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
        let refresh_epoch = self
            .refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire);
        if refresh_epoch & REFRESH_PUBLISHING_BIT != 0
            || refresh_epoch & REFRESH_RECONCILING_BIT == 0
            || refresh_epoch & REFRESH_GENERATION_MASK != refresh_generation
        {
            drop(refresh_state);
            self.abandon_refresh(Some(refresh_generation));
            return false;
        }
        if self
            .refresh_epoch
            .compare_exchange(
                refresh_epoch,
                refresh_generation | REFRESH_PUBLISHING_BIT,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            drop(refresh_state);
            self.abandon_refresh(Some(refresh_generation));
            return false;
        }
        // A mutation cannot advance the epoch while this publishing lease is
        // held, so the snapshot built for this generation cannot be installed
        // after a newer mutation becomes visible.
        *self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned") = Some(Arc::new(snapshot));
        let wake_waiters = std::mem::take(&mut refresh_state.wake_waiters);
        refresh_state.pending = false;
        self.refresh_epoch
            .store(refresh_generation, std::sync::atomic::Ordering::Release);
        drop(refresh_state);
        if wake_waiters
            || self
                .deferred_availability_wake
                .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            self.publish_availability_if_ready(publish_availability);
        }
        true
    }

    pub(crate) fn invalidate(&self) {
        self.advance_refresh_epoch();
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
        *self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned") = None;
        // Stay fail-closed and retain a queued recovery wake until a later
        // reconciler pass installs a current snapshot.
        refresh_state.pending = true;
    }

    pub(crate) fn refresh_pending(&self) -> bool {
        if self
            .refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire)
            & (REFRESH_PENDING_BIT
                | REFRESH_PUBLISHING_BIT
                | REFRESH_RECONCILING_BIT
                | REFRESH_COALESCED_SUCCESSOR_BIT)
            != 0
        {
            return true;
        }
        self.refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned")
            .pending
    }

    pub(crate) fn refresh_generation(&self) -> u64 {
        self.refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire)
            & REFRESH_GENERATION_MASK
    }

    fn immediate_successor_pending(&self) -> bool {
        let epoch = self
            .refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire);
        epoch & (REFRESH_PENDING_BIT | REFRESH_RECONCILING_BIT | REFRESH_COALESCED_SUCCESSOR_BIT)
            == (REFRESH_PENDING_BIT | REFRESH_COALESCED_SUCCESSOR_BIT)
    }

    pub(crate) fn publish_availability_if_ready(&self, publish_availability: impl Fn()) {
        loop {
            let epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if epoch
                & (REFRESH_PENDING_BIT
                    | REFRESH_PUBLISHING_BIT
                    | REFRESH_RECONCILING_BIT
                    | REFRESH_COALESCED_SUCCESSOR_BIT)
                != 0
            {
                // A release during a snapshot rebuild must wake only after its
                // fenced snapshot is current. Retain the wake across both the
                // pending and publishing phases instead of dropping it.
                self.deferred_availability_wake
                    .store(true, std::sync::atomic::Ordering::Release);
                // If the lease ended before the deferred flag was recorded,
                // retry immediately so this release cannot be left for a
                // future, unrelated refresh cycle.
                if self
                    .refresh_epoch
                    .load(std::sync::atomic::Ordering::Acquire)
                    == epoch
                {
                    return;
                }
                continue;
            }
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    epoch | REFRESH_PUBLISHING_BIT,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                publish_availability();
                self.refresh_epoch
                    .store(epoch, std::sync::atomic::Ordering::Release);
                if !self
                    .deferred_availability_wake
                    .swap(false, std::sync::atomic::Ordering::AcqRel)
                {
                    return;
                }
            }
        }
    }

    fn abandon_refresh(&self, expected_generation: Option<u64>) {
        loop {
            let epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if epoch & REFRESH_RECONCILING_BIT == 0 {
                return;
            }
            let stale_generation = expected_generation
                .is_some_and(|generation| epoch & REFRESH_GENERATION_MASK != generation);
            let successor = if stale_generation && epoch & REFRESH_PENDING_BIT != 0 {
                REFRESH_COALESCED_SUCCESSOR_BIT
            } else {
                0
            };
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    (epoch & !REFRESH_RECONCILING_BIT) | successor,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    fn advance_refresh_epoch(&self) -> bool {
        loop {
            let epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if epoch & REFRESH_PUBLISHING_BIT != 0 {
                std::hint::spin_loop();
                continue;
            }
            if epoch & REFRESH_PENDING_BIT != 0 {
                return false;
            }
            let next_generation =
                (epoch & REFRESH_GENERATION_MASK).wrapping_add(1) & REFRESH_GENERATION_MASK;
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    next_generation
                        | REFRESH_PENDING_BIT
                        | (epoch & (REFRESH_RECONCILING_BIT | REFRESH_COALESCED_SUCCESSOR_BIT)),
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }
}

async fn try_refresh_pool_routing_snapshot(state: &AppState) -> Result<bool> {
    let Some(refresh_generation) = state.pool_routing_snapshot.begin_refresh() else {
        return Ok(false);
    };
    let mut refresh_lease =
        PoolRoutingSnapshotRefreshLease::new(&state.pool_routing_snapshot, refresh_generation);
    let snapshot = async {
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
    Ok(PoolRoutingSnapshot {
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
        })
    }
    .await;
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => return Err(error),
    };
    Ok(refresh_lease.complete(snapshot, || state.pool_routing_availability.publish()))
}

pub(crate) async fn refresh_pool_routing_snapshot(state: &AppState) -> Result<()> {
    loop {
        if try_refresh_pool_routing_snapshot(state).await? {
            return Ok(());
        }
        // An explicit refresh is a completion barrier for startup and mutation
        // callers. The background reconciler uses the one-shot form below.
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn reconcile_pool_routing_snapshot(state: &AppState) -> Result<()> {
    // An event that races a normal build fences it and earns one immediate
    // successor. Further races are left for the 60-second reconciliation.
    for _ in 0..2 {
        if try_refresh_pool_routing_snapshot(state).await?
            || !state.pool_routing_snapshot.immediate_successor_pending()
        {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod snapshot_store_tests {
    use super::*;

    fn empty_snapshot() -> PoolRoutingSnapshot {
        PoolRoutingSnapshot {
            candidates: HashMap::new(),
            candidate_order: Vec::new(),
            accounts: HashMap::new(),
            effective_rules: HashMap::new(),
            node_shunt_assignments: UpstreamAccountNodeShuntAssignments::default(),
            model_routes: HashMap::new(),
            route_binding_failure_penalties: HashMap::new(),
            transport_decode_sticky_escape_states: HashMap::new(),
            group_metadata: HashMap::new(),
            sticky_routes: HashMap::new(),
            sticky_model_routes: HashMap::new(),
            sticky_generations: HashMap::new(),
            sticky_model_generations: HashMap::new(),
            cache_hit_protection: CacheHitProtectionSettings {
                enabled: false,
                low_hit_rate_threshold_percent: 10,
                overflow_mode: CacheHitOverflowMode::Queue,
            },
        }
    }

    #[test]
    fn refresh_request_advances_generation_before_waiting_for_refresh_state() {
        let store = Arc::new(PoolRoutingSnapshotStore::new());
        let previous_generation = store.refresh_generation();
        let refresh_state = store
            .refresh_state
            .lock()
            .expect("lock refresh state for test");
        let requested_store = store.clone();
        let requested = std::thread::spawn(move || requested_store.request_refresh());

        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while store.refresh_generation() == previous_generation {
            assert!(
                std::time::Instant::now() < deadline,
                "refresh request should advance its generation before waiting on refresh state"
            );
            std::thread::yield_now();
        }
        assert!(
            store.refresh_pending(),
            "refresh request should mark pending before waiting on refresh state"
        );
        drop(refresh_state);
        requested
            .join()
            .expect("refresh request thread should join");
    }

    #[test]
    fn stale_refresh_cannot_install_after_a_newer_mutation() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let stale_generation = store
            .begin_refresh()
            .expect("queued refresh should claim its reconciliation lease");
        store.request_refresh();

        assert!(
            !store.complete_refresh(stale_generation, empty_snapshot(), || {
                panic!("stale refresh must not wake waiters")
            }),
            "a snapshot loaded before a newer mutation must not install"
        );
        assert!(store.current().is_none());
        assert!(store.refresh_pending());
    }

    #[test]
    fn successor_event_cannot_install_a_snapshot_that_predates_it() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let first_generation = store
            .begin_refresh()
            .expect("queued refresh should claim its reconciliation lease");

        store.request_refresh_and_wake_waiters();
        let successor_generation = store.refresh_generation();
        store.request_refresh_and_wake_waiters();
        assert_eq!(
            store.refresh_generation(),
            successor_generation,
            "additional outage events must not keep invalidating the queued successor"
        );
        assert!(
            !store.complete_refresh(first_generation, empty_snapshot(), || {}),
            "the build predating the first new event must not install"
        );

        let successor = store
            .begin_refresh()
            .expect("one successor should remain after the stale build exits");
        assert_eq!(successor, successor_generation);
        store.request_refresh_and_wake_waiters();
        let deferred_generation = store.refresh_generation();
        assert_ne!(
            store.refresh_generation(),
            successor_generation,
            "an event during the successor must fence its stale snapshot"
        );
        assert!(
            !store.complete_refresh(successor, empty_snapshot(), || {}),
            "a successor cannot publish across a newer mutation"
        );
        assert!(store.current().is_none());
        assert!(
            store.refresh_pending(),
            "the later event remains queued for low-frequency reconciliation"
        );

        let deferred = store
            .begin_refresh()
            .expect("the deferred reconciliation should claim the queued generation");
        assert_eq!(deferred, deferred_generation);
        assert!(store.complete_refresh(deferred, empty_snapshot(), || {}));
        assert!(
            !store.refresh_pending(),
            "a quiet deferred reconciliation should restore the ready state"
        );
    }

    #[test]
    fn initial_refresh_can_install_without_a_mutation_event() {
        let store = PoolRoutingSnapshotStore::new();
        let initial_generation = store
            .begin_refresh()
            .expect("initial refresh should claim its reconciliation lease");
        let published_availability = std::cell::Cell::new(false);

        assert!(
            store.complete_refresh(initial_generation, empty_snapshot(), || {
                published_availability.set(true);
            })
        );
        assert!(store.current().is_some());
        assert!(!store.refresh_pending());
        assert!(
            !published_availability.get(),
            "an initial refresh must not publish a waiter wake without a queued capacity event"
        );
    }

    #[test]
    fn pending_refresh_wakes_waiters_only_after_the_snapshot_installs() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh_and_wake_waiters();
        let generation = store
            .begin_refresh()
            .expect("pending refresh should claim its reconciliation lease");
        let published_availability = std::cell::Cell::new(false);

        assert!(store.complete_refresh(generation, empty_snapshot(), || {
            published_availability.set(true);
        }));
        assert!(published_availability.get());
        assert!(!store.refresh_pending());
    }

    #[test]
    fn availability_release_deferred_while_pending_wakes_after_snapshot_install() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let generation = store
            .begin_refresh()
            .expect("pending refresh should claim its reconciliation lease");
        let publish_count = std::cell::Cell::new(0);

        store.publish_availability_if_ready(|| publish_count.set(publish_count.get() + 1));
        assert_eq!(
            publish_count.get(),
            0,
            "a pending snapshot must fence capacity wakes"
        );

        assert!(store.complete_refresh(generation, empty_snapshot(), || {
            publish_count.set(publish_count.get() + 1);
        }));
        assert_eq!(
            publish_count.get(),
            1,
            "the installed snapshot must replay the deferred capacity wake"
        );
    }

    #[test]
    fn availability_release_deferred_while_publishing_is_replayed() {
        let store = PoolRoutingSnapshotStore::new();
        let publish_count = std::cell::Cell::new(0);

        store.publish_availability_if_ready(|| {
            publish_count.set(publish_count.get() + 1);
            if publish_count.get() == 1 {
                // This nested release sees the publishing lease. Its wake must
                // be replayed after the lease rather than being silently lost.
                store.publish_availability_if_ready(|| {});
            }
        });

        assert_eq!(
            publish_count.get(),
            2,
            "a capacity release during a publishing lease must get its own wake"
        );
    }

    #[test]
    fn cancelled_refresh_lease_releases_the_reconciliation_bit() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let generation = store
            .begin_refresh()
            .expect("queued refresh should claim its reconciliation lease");

        drop(PoolRoutingSnapshotRefreshLease::new(&store, generation));

        assert!(
            store.begin_refresh().is_some(),
            "a cancelled refresh must not strand the reconciliation lease"
        );
    }
}
