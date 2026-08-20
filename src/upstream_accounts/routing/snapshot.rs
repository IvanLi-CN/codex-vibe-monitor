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
    committed_failure_fences: HashSet<(i64, String)>,
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
    fn with_committed_model_failure_fence(&self, account_id: i64, model: &str) -> Option<Self> {
        let model = normalized_model_key(Some(model))?;
        let mut snapshot = self.clone();
        snapshot
            .committed_failure_fences
            .insert((account_id, model));
        Some(snapshot)
    }

    fn without_failed_account(&self, account_id: i64) -> Self {
        let mut snapshot = self.clone();
        snapshot.candidates.remove(&account_id);
        snapshot
            .candidate_order
            .retain(|candidate_id| *candidate_id != account_id);
        snapshot.accounts.remove(&account_id);
        snapshot.effective_rules.remove(&account_id);
        snapshot
            .model_routes
            .retain(|(candidate_id, _), _| *candidate_id != account_id);
        snapshot
            .committed_failure_fences
            .retain(|(candidate_id, _)| *candidate_id != account_id);
        snapshot
    }

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
                if self
                    .committed_failure_fences
                    .contains(&(*account_id, model.clone()))
                {
                    return Some((*account_id, ModelRoutePenalty::Excluded));
                }
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
// A stale normal build gets one immediate successor. A capacity-increasing
// event which races that successor can schedule one additional follow-up so
// waiters do not wait for the 60-second reconciliation. Other mutations stay
// fenced and deferred, avoiding an unbounded SQL rebuild loop.
const REFRESH_COALESCED_SUCCESSOR_BIT: u64 = 1 << 60;
const REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT: u64 = 1 << 59;
const REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT: u64 = 1 << 58;
const REFRESH_GENERATION_MASK: u64 = !(REFRESH_PENDING_BIT
    | REFRESH_PUBLISHING_BIT
    | REFRESH_RECONCILING_BIT
    | REFRESH_COALESCED_SUCCESSOR_BIT
    | REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT
    | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT);

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
        self.current_with_generation()
            .map(|(snapshot, _generation)| snapshot)
    }

    pub(crate) fn current_with_generation(&self) -> Option<(Arc<PoolRoutingSnapshot>, u64)> {
        let epoch_before = self
            .refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire);
        if epoch_before & REFRESH_PENDING_BIT != 0 {
            return None;
        }
        let snapshot = self
            .snapshot
            .read()
            .expect("pool routing snapshot lock poisoned");
        let current = snapshot.clone();
        // `request_refresh_inner` advances the epoch before it acquires this
        // lock to discard the old value. The second epoch read rejects a
        // mutation racing the clone, so request-time routing cannot keep a
        // stale Arc across the event fence.
        let epoch_after = self
            .refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire);
        if epoch_before != epoch_after || epoch_after & REFRESH_PENDING_BIT != 0 {
            return None;
        }
        current.map(|snapshot| (snapshot, epoch_after & REFRESH_GENERATION_MASK))
    }

    pub(crate) fn generation_is_current(&self, expected_generation: u64) -> bool {
        let epoch = self
            .refresh_epoch
            .load(std::sync::atomic::Ordering::Acquire);
        epoch & REFRESH_PENDING_BIT == 0 && epoch & REFRESH_GENERATION_MASK == expected_generation
    }

    pub(crate) fn request_refresh(&self) {
        self.request_refresh_inner(false, false);
    }

    pub(crate) fn request_refresh_and_wake_waiters(&self, publish_availability: impl FnOnce()) {
        self.request_refresh_inner(true, false);
        publish_availability();
    }

    pub(crate) fn request_refresh_and_defer_availability_wake(&self) {
        self.request_refresh_inner(true, false);
    }

    /// Schedules one replacement after a persisted capacity recovery without
    /// fencing an established route. Waiters are notified only after that
    /// replacement is installed, so they cannot retry against the old view.
    pub(crate) fn request_recovery_refresh_and_defer_availability_wake(&self) {
        let mut refresh_state = self
            .refresh_state
            .lock()
            .expect("pool routing snapshot refresh lock poisoned");
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
            drop(refresh_state);
            self.request_refresh_inner(true, true);
            return;
        }
        if refresh_state.pending {
            refresh_state.wake_waiters = true;
            return;
        }
        refresh_state.pending = true;
        refresh_state.wake_waiters = true;
        drop(refresh_state);
        self.refresh_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }

    /// Applies a committed model failure immediately to the in-memory routing
    /// view. The periodic reconciler later restores the database's precise
    /// model-health state, but the request that observed the failure can
    /// immediately fail over without reopening a candidate SQL path.
    ///
    /// Returns false when there is no current view to safely patch. Callers
    /// must then retain the normal fail-closed refresh fence.
    pub(crate) fn apply_committed_model_failure_fence(&self, account_id: i64, model: &str) -> bool {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned");

        loop {
            let epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if epoch & (REFRESH_PENDING_BIT | REFRESH_PUBLISHING_BIT) != 0 {
                return false;
            }
            let Some(current) = snapshot.as_ref() else {
                return false;
            };
            let Some(next_snapshot) = current.with_committed_model_failure_fence(account_id, model)
            else {
                return false;
            };
            if next_snapshot.committed_failure_fences == current.committed_failure_fences {
                return true;
            }

            let next_generation =
                (epoch & REFRESH_GENERATION_MASK).wrapping_add(1) & REFRESH_GENERATION_MASK;
            let next_epoch = next_generation
                | (epoch
                    & (REFRESH_RECONCILING_BIT
                        | REFRESH_COALESCED_SUCCESSOR_BIT
                        | REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT
                        | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT));
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    next_epoch,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                // The write lock remains held across the epoch transition.
                // Readers that sampled the prior generation either see this
                // replacement or reject on their second epoch read.
                *snapshot = Some(Arc::new(next_snapshot));
                return true;
            }
        }
    }

    /// Applies a committed account cooldown or hard-unavailable mutation to
    /// the in-memory view before releasing a reservation.
    pub(crate) fn apply_committed_account_failure_fence(&self, account_id: i64) -> bool {
        let mut snapshot = self
            .snapshot
            .write()
            .expect("pool routing snapshot lock poisoned");

        loop {
            let epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if epoch & (REFRESH_PENDING_BIT | REFRESH_PUBLISHING_BIT) != 0 {
                return false;
            }
            let Some(current) = snapshot.as_ref() else {
                return false;
            };
            if current.account(account_id).is_none() {
                return true;
            }
            let next_snapshot = current.without_failed_account(account_id);
            let next_generation =
                (epoch & REFRESH_GENERATION_MASK).wrapping_add(1) & REFRESH_GENERATION_MASK;
            let next_epoch = next_generation
                | (epoch
                    & (REFRESH_RECONCILING_BIT
                        | REFRESH_COALESCED_SUCCESSOR_BIT
                        | REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT
                        | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT));
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    next_epoch,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                *snapshot = Some(Arc::new(next_snapshot));
                return true;
            }
        }
    }

    fn request_refresh_inner(&self, wake_waiters: bool, ensure_recovery_successor: bool) {
        // Atomically fence an older reconciler and make the pending state
        // visible before waiting for refresh_state. This blocks stale capacity
        // wakeups while a mutation is queued behind a completing refresh.
        let queued_refresh = self.advance_refresh_epoch(wake_waiters, ensure_recovery_successor);
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
            // Coordinate with non-fencing recovery scheduling. A build that
            // starts after it takes this lock observes the committed recovery;
            // a recovery that arrives after this lease starts upgrades to the
            // existing fenced successor path instead of installing stale data.
            let _refresh_state = self
                .refresh_state
                .lock()
                .expect("pool routing snapshot refresh lock poisoned");
            let refresh_epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if refresh_epoch & REFRESH_PUBLISHING_BIT != 0 {
                continue;
            }
            if refresh_epoch & REFRESH_RECONCILING_BIT != 0 {
                return None;
            }
            let refresh_generation = refresh_epoch & REFRESH_GENERATION_MASK;
            let burst_state = refresh_epoch
                & (REFRESH_COALESCED_SUCCESSOR_BIT | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT);
            if self
                .refresh_epoch
                .compare_exchange(
                    refresh_epoch,
                    refresh_generation | REFRESH_RECONCILING_BIT | burst_state,
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
            || refresh_epoch & REFRESH_PENDING_BIT != 0
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
        self.advance_refresh_epoch(false, false);
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
                // A release must wake current waiters immediately, even when
                // the candidate snapshot is fenced cold. The deferred signal
                // gives those waiters another chance after the next safe
                // snapshot install without coupling availability SSE traffic
                // to an immediate SQL reconciliation.
                self.deferred_availability_wake
                    .store(true, std::sync::atomic::Ordering::Release);
                publish_availability();
                return;
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
            let successor_was_queued = epoch & REFRESH_COALESCED_SUCCESSOR_BIT != 0;
            let successor = if stale_generation && epoch & REFRESH_PENDING_BIT != 0 {
                REFRESH_COALESCED_SUCCESSOR_BIT
            } else {
                0
            };
            let availability_followup = epoch & REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT != 0;
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    (epoch & !(REFRESH_RECONCILING_BIT | REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT))
                        | successor,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
            {
                if (!successor_was_queued && successor != 0) || availability_followup {
                    self.refresh_tx.send_modify(|generation| {
                        *generation = generation.wrapping_add(1);
                    });
                }
                return;
            }
        }
    }

    fn advance_refresh_epoch(&self, wake_waiters: bool, ensure_recovery_successor: bool) -> bool {
        loop {
            let epoch = self
                .refresh_epoch
                .load(std::sync::atomic::Ordering::Acquire);
            if epoch & REFRESH_PUBLISHING_BIT != 0 {
                std::hint::spin_loop();
                continue;
            }
            let successor_queued = epoch & REFRESH_COALESCED_SUCCESSOR_BIT != 0;
            let availability_followup_used = epoch & REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT != 0;
            if epoch & REFRESH_PENDING_BIT != 0 {
                if ensure_recovery_successor && !successor_queued {
                    let next_generation =
                        (epoch & REFRESH_GENERATION_MASK).wrapping_add(1) & REFRESH_GENERATION_MASK;
                    let next_epoch = next_generation
                        | REFRESH_PENDING_BIT
                        | (epoch
                            & (REFRESH_RECONCILING_BIT | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT));
                    if self
                        .refresh_epoch
                        .compare_exchange(
                            epoch,
                            next_epoch,
                            std::sync::atomic::Ordering::AcqRel,
                            std::sync::atomic::Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        // A recovery is capacity-bearing. Make the active
                        // build stale so its lease queues one successor; if
                        // no lease owns the pending work, wake the reconciler
                        // directly. Ordinary events retain the existing
                        // bounded coalescing behavior.
                        return epoch & REFRESH_RECONCILING_BIT == 0;
                    }
                    continue;
                }
                if wake_waiters && successor_queued && !availability_followup_used {
                    let next_epoch = epoch
                        | REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT
                        | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT;
                    if self
                        .refresh_epoch
                        .compare_exchange(
                            epoch,
                            next_epoch,
                            std::sync::atomic::Ordering::AcqRel,
                            std::sync::atomic::Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return epoch & REFRESH_RECONCILING_BIT == 0;
                    }
                    continue;
                }
                return false;
            }
            if successor_queued {
                let availability_followup = wake_waiters && !availability_followup_used;
                let availability_followup_bits = if availability_followup {
                    REFRESH_AVAILABILITY_FOLLOWUP_QUEUED_BIT
                        | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT
                } else {
                    0
                };
                let next_epoch = epoch | REFRESH_PENDING_BIT | availability_followup_bits;
                if self
                    .refresh_epoch
                    .compare_exchange(
                        epoch,
                        next_epoch,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Acquire,
                    )
                    .is_ok()
                {
                    // A successor has already been allowed for this burst.
                    // A capacity wake earns one independently scheduled
                    // follow-up; other events wait for the 60-second tick.
                    return availability_followup && epoch & REFRESH_RECONCILING_BIT == 0;
                }
                continue;
            }
            let next_generation =
                (epoch & REFRESH_GENERATION_MASK).wrapping_add(1) & REFRESH_GENERATION_MASK;
            if self
                .refresh_epoch
                .compare_exchange(
                    epoch,
                    next_generation
                        | REFRESH_PENDING_BIT
                        | (epoch
                            & (REFRESH_RECONCILING_BIT
                                | REFRESH_COALESCED_SUCCESSOR_BIT
                                | REFRESH_AVAILABILITY_FOLLOWUP_USED_BIT)),
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
            committed_failure_fences: HashSet::new(),
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
    // `abandon_refresh` schedules a bounded successor only after the stale
    // build has released its lease. Keeping this one-shot avoids consuming a
    // watch event while the build is still active and prevents hot rebuilds.
    let _ = try_refresh_pool_routing_snapshot(state).await?;
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
            committed_failure_fences: HashSet::new(),
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
    fn mutation_fence_makes_current_snapshot_cold_before_its_write_lock() {
        let store = Arc::new(PoolRoutingSnapshotStore::new());
        let initial_generation = store
            .begin_refresh()
            .expect("initial refresh should claim its reconciliation lease");
        assert!(store.complete_refresh(initial_generation, empty_snapshot(), || {}));

        // Hold the snapshot read lock so the mutation can advance its epoch
        // but cannot clear the old Arc. `current` must observe that fence
        // without waiting for or returning the stale snapshot.
        let snapshot_read = store
            .snapshot
            .read()
            .expect("lock snapshot for mutation fence test");
        let requested_store = store.clone();
        let requested = std::thread::spawn(move || requested_store.request_refresh());
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while !store.refresh_pending() {
            assert!(
                std::time::Instant::now() < deadline,
                "mutation should set the refresh fence before waiting for the snapshot lock"
            );
            std::thread::yield_now();
        }
        assert!(
            store.current().is_none(),
            "request-time routing must fail closed once the mutation fence is visible"
        );
        drop(snapshot_read);
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
    fn successor_capacity_event_gets_one_immediate_followup() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let first_generation = store
            .begin_refresh()
            .expect("queued refresh should claim its reconciliation lease");

        store.request_refresh();
        let successor_generation = store.refresh_generation();
        assert!(
            !store.complete_refresh(first_generation, empty_snapshot(), || {}),
            "the build predating the first new event must not install"
        );

        let successor = store
            .begin_refresh()
            .expect("one successor should remain after the stale build exits");
        assert_eq!(successor, successor_generation);
        let refreshes = store.subscribe_refresh();
        store.request_refresh_and_wake_waiters(|| {});
        assert_eq!(
            store.refresh_generation(),
            successor_generation,
            "the capacity event shares the successor generation and fences its stale build"
        );
        assert!(
            !store.complete_refresh(successor, empty_snapshot(), || {}),
            "a successor cannot publish across a newer mutation"
        );
        assert!(
            refreshes
                .has_changed()
                .expect("refresh watch should remain open"),
            "the capacity event must schedule one immediate follow-up after the successor exits"
        );
        assert!(store.current().is_none());

        let followup = store
            .begin_refresh()
            .expect("the immediate capacity follow-up should claim the queued generation");
        assert_eq!(followup, successor_generation);
        assert!(store.complete_refresh(followup, empty_snapshot(), || {}));
        assert!(
            !store.refresh_pending(),
            "a quiet immediate follow-up should restore the ready state"
        );
    }

    #[test]
    fn successor_non_capacity_event_waits_for_the_low_frequency_reconcile() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let first_generation = store
            .begin_refresh()
            .expect("queued refresh should claim its reconciliation lease");
        store.request_refresh();
        assert!(!store.complete_refresh(first_generation, empty_snapshot(), || {}));

        let successor = store
            .begin_refresh()
            .expect("one successor should remain after the stale build exits");
        let refreshes = store.subscribe_refresh();
        store.request_refresh();
        assert!(!store.complete_refresh(successor, empty_snapshot(), || {}));

        assert!(
            !refreshes
                .has_changed()
                .expect("refresh watch should remain open"),
            "ordinary successor races must not schedule another immediate rebuild"
        );
        assert!(store.refresh_pending());
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
    fn committed_failure_fence_only_excludes_the_exact_model() {
        let store = PoolRoutingSnapshotStore::new();
        let initial_generation = store
            .begin_refresh()
            .expect("initial refresh should claim its reconciliation lease");
        assert!(store.complete_refresh(initial_generation, empty_snapshot(), || {}));

        assert!(store.apply_committed_model_failure_fence(7, "model-a"));
        let snapshot = store.current().expect("patched snapshot remains available");
        assert_eq!(
            snapshot.model_route_penalties(&[7], Some("model-a")),
            HashMap::from([(7, ModelRoutePenalty::Excluded)]),
            "the failed model must be fenced immediately",
        );
        assert!(
            snapshot
                .model_route_penalties(&[7], Some("model-b"))
                .is_empty(),
            "a model-a failure must not exclude model-b for the same account",
        );
    }

    #[test]
    fn pending_refresh_wakes_waiters_only_after_the_snapshot_installs() {
        let store = PoolRoutingSnapshotStore::new();
        let immediately_published = std::cell::Cell::new(false);
        store.request_refresh_and_wake_waiters(|| immediately_published.set(true));
        assert!(
            immediately_published.get(),
            "a capacity-bearing mutation must immediately signal existing waiters"
        );
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
    fn failure_fence_defers_waiters_until_the_snapshot_installs() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh_and_defer_availability_wake();
        let generation = store
            .begin_refresh()
            .expect("failure fence should claim its reconciliation lease");
        let published_availability = std::cell::Cell::new(false);

        assert!(store.complete_refresh(generation, empty_snapshot(), || {
            published_availability.set(true);
        }));
        assert!(published_availability.get());
        assert!(!store.refresh_pending());
    }

    #[test]
    fn recovery_replaces_the_snapshot_before_waking_waiters() {
        let store = PoolRoutingSnapshotStore::new();
        let initial_generation = store
            .begin_refresh()
            .expect("initial refresh should claim its reconciliation lease");
        assert!(store.complete_refresh(initial_generation, empty_snapshot(), || {}));
        let current_generation = store.refresh_generation();
        let published_availability = std::cell::Cell::new(false);

        store.request_recovery_refresh_and_defer_availability_wake();

        assert!(
            store.current().is_some(),
            "a capacity recovery must preserve the established view until its replacement is ready"
        );
        assert_eq!(
            store.refresh_generation(),
            current_generation,
            "a non-fencing recovery must not invalidate established reservation generations"
        );
        assert!(store.refresh_pending());
        assert!(
            !published_availability.get(),
            "waiters must not wake against the pre-recovery snapshot"
        );

        let recovery_generation = store
            .begin_refresh()
            .expect("the scheduled recovery should claim one reconciliation lease");
        assert_eq!(recovery_generation, current_generation);
        assert!(
            store.complete_refresh(recovery_generation, empty_snapshot(), || {
                published_availability.set(true);
            })
        );
        assert!(
            published_availability.get(),
            "the replacement installation must replay the deferred recovery wake"
        );
    }

    #[test]
    fn recovery_during_an_active_refresh_fences_the_stale_build() {
        let store = PoolRoutingSnapshotStore::new();
        let initial_generation = store
            .begin_refresh()
            .expect("initial refresh should claim its reconciliation lease");
        assert!(store.complete_refresh(initial_generation, empty_snapshot(), || {}));
        let mut refreshes = store.subscribe_refresh();

        store.request_refresh();
        assert!(
            refreshes
                .has_changed()
                .expect("the initial refresh must signal the reconciler")
        );
        refreshes.borrow_and_update();
        let stale_generation = store
            .begin_refresh()
            .expect("the queued refresh should claim its reconciliation lease");
        store.request_recovery_refresh_and_defer_availability_wake();

        assert!(
            store.current().is_none(),
            "a recovery racing a refresh must fence the stale build rather than install its old view"
        );
        assert!(
            !store.complete_refresh(stale_generation, empty_snapshot(), || {
                panic!("a fenced stale refresh must not wake waiters")
            }),
            "the refresh that predated the recovery must not install"
        );
        assert!(
            refreshes
                .has_changed()
                .expect("the capacity recovery must schedule one successor"),
            "a deferred recovery wake must not wait for the periodic ticker"
        );
        refreshes.borrow_and_update();
        let successor_generation = store
            .begin_refresh()
            .expect("the recovery successor should claim its reconciliation lease");
        let published_availability = std::cell::Cell::new(false);
        assert!(
            store.complete_refresh(successor_generation, empty_snapshot(), || {
                published_availability.set(true);
            })
        );
        assert!(
            published_availability.get(),
            "the recovery successor must publish the deferred waiter wake"
        );
    }

    #[test]
    fn availability_release_wakes_immediately_and_again_after_snapshot_install() {
        let store = PoolRoutingSnapshotStore::new();
        store.request_refresh();
        let generation = store
            .begin_refresh()
            .expect("pending refresh should claim its reconciliation lease");
        let publish_count = std::cell::Cell::new(0);

        store.publish_availability_if_ready(|| publish_count.set(publish_count.get() + 1));
        assert_eq!(
            publish_count.get(),
            1,
            "a release must wake waiters immediately while the stale snapshot stays cold"
        );

        assert!(store.complete_refresh(generation, empty_snapshot(), || {
            publish_count.set(publish_count.get() + 1);
        }));
        assert_eq!(
            publish_count.get(),
            2,
            "the installed snapshot must replay the deferred wake for a fresh selection"
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
