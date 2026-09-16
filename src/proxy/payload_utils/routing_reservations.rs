use super::*;
pub(crate) fn parse_proxy_capture_summary(payload: Option<&str>) -> (ProxyCaptureTarget, bool) {
    let mut target = ProxyCaptureTarget::Responses;
    let mut is_stream = false;

    let Some(raw) = payload else {
        return (target, is_stream);
    };
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return (target, is_stream);
    };

    if let Some(endpoint) = value.get("endpoint").and_then(|v| v.as_str()) {
        target = ProxyCaptureTarget::from_endpoint(endpoint);
    }
    if let Some(stream) = value.get("isStream").and_then(|v| v.as_bool()) {
        is_stream = stream;
    }

    (target, is_stream)
}

pub(crate) fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

pub(crate) fn percentile_sorted_f64(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }
    let clamped = p.clamp(0.0, 1.0);
    let rank = clamped * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted_values[lower];
    }
    let weight = rank - lower as f64;
    sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * weight
}

pub(crate) fn next_proxy_request_id() -> u64 {
    NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) const PROXY_INVOKE_ID_LENGTH: usize = 10;
pub(crate) const PROXY_INVOKE_ID_GENERATION_ATTEMPTS: usize = 5;
pub(crate) const PROXY_INVOKE_ID_ALPHABET: [char; 31] = [
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'M', 'N', 'P', 'Q', 'R', 'S', 'T', 'U', 'V',
    'W', 'X', 'Y', 'Z', '2', '3', '4', '5', '6', '7', '8', '9',
];

pub(crate) fn generate_proxy_invoke_id() -> String {
    nanoid::nanoid!(PROXY_INVOKE_ID_LENGTH, &PROXY_INVOKE_ID_ALPHABET)
}

#[cfg(test)]
pub(crate) fn proxy_invoke_id_has_short_format(value: &str) -> bool {
    value.len() == PROXY_INVOKE_ID_LENGTH
        && value
            .chars()
            .all(|ch| PROXY_INVOKE_ID_ALPHABET.contains(&ch))
}

pub(crate) async fn proxy_invoke_id_exists(pool: &Pool<Sqlite>, invoke_id: &str) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM codex_invocations
            WHERE invoke_id = ?1
            LIMIT 1
        )
        "#,
    )
    .bind(invoke_id)
    .fetch_one(pool)
    .await?;
    Ok(exists != 0)
}

pub(crate) async fn generate_unique_proxy_invoke_id(pool: &Pool<Sqlite>) -> String {
    for _ in 0..PROXY_INVOKE_ID_GENERATION_ATTEMPTS {
        let candidate = generate_proxy_invoke_id();
        match proxy_invoke_id_exists(pool, &candidate).await {
            Ok(false) => return candidate,
            Ok(true) => continue,
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to check generated proxy invoke id uniqueness; using generated id"
                );
                return candidate;
            }
        }
    }

    let fallback = generate_proxy_invoke_id();
    warn!(
        attempts = PROXY_INVOKE_ID_GENERATION_ATTEMPTS,
        fallback_invoke_id = %fallback,
        "generated proxy invoke id collided repeatedly; using final fallback id"
    );
    fallback
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingReservation {
    pub(crate) account_id: i64,
    pub(crate) model: Option<String>,
    pub(crate) proxy_key: Option<String>,
    #[allow(dead_code)]
    pub(crate) created_at: Instant,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PoolRoutingReservationSnapshot {
    counts_by_account: HashMap<i64, i64>,
    proxy_keys_by_account: HashMap<i64, HashSet<String>>,
    reserved_proxy_keys: HashSet<String>,
}

impl PoolRoutingReservationSnapshot {
    pub(crate) fn count_for_account(&self, account_id: i64) -> i64 {
        self.counts_by_account
            .get(&account_id)
            .copied()
            .unwrap_or_default()
    }

    pub(crate) fn pinned_proxy_keys_for_account(
        &self,
        account_id: i64,
        valid_proxy_keys: &[String],
        occupied_proxy_keys: &HashSet<String>,
    ) -> Vec<String> {
        let Some(proxy_keys) = self.proxy_keys_by_account.get(&account_id) else {
            return Vec::new();
        };
        valid_proxy_keys
            .iter()
            .filter(|proxy_key| {
                proxy_keys.contains(proxy_key.as_str())
                    && !occupied_proxy_keys.contains(proxy_key.as_str())
            })
            .cloned()
            .collect()
    }

    pub(crate) fn reserved_proxy_keys_for_group(
        &self,
        valid_proxy_keys: &[String],
    ) -> HashSet<String> {
        let valid_proxy_keys = valid_proxy_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        self.reserved_proxy_keys
            .iter()
            .filter(|proxy_key| valid_proxy_keys.contains(proxy_key.as_str()))
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
pub(crate) struct PoolRoutingReservationDropGuard {
    state: Arc<AppState>,
    reservation_key: String,
    active: bool,
    publish_on_drop: bool,
}

impl PoolRoutingReservationDropGuard {
    pub(crate) fn new(state: Arc<AppState>, reservation_key: String) -> Self {
        Self {
            state,
            reservation_key,
            active: true,
            publish_on_drop: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.active = false;
    }

    pub(crate) fn suppress_availability_on_drop(&mut self) {
        self.publish_on_drop = false;
    }

    pub(crate) fn restore_availability_on_drop(&mut self) {
        self.publish_on_drop = true;
    }

    pub(crate) async fn fence_failure<T, E, F>(&mut self, persist_failure: F) -> Result<T, E>
    where
        F: std::future::Future<Output = Result<T, E>>,
    {
        self.suppress_availability_on_drop();
        match persist_failure.await {
            Ok(value) => {
                self.restore_availability_on_drop();
                Ok(value)
            }
            // A failed write never establishes a routing fence. Keep the eventual
            // guard release silent so a waiter cannot immediately reselect it.
            Err(err) => Err(err),
        }
    }
}

impl Drop for PoolRoutingReservationDropGuard {
    fn drop(&mut self) {
        if self.active {
            release_pool_routing_reservation_with_availability(
                self.state.as_ref(),
                &self.reservation_key,
                self.publish_on_drop,
            );
        }
    }
}

pub(crate) fn build_pool_routing_reservation_key(proxy_request_id: u64) -> String {
    format!("pool-route-{proxy_request_id}")
}

pub(crate) fn pool_routing_reservation_count(state: &AppState, account_id: i64) -> i64 {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations
        .values()
        .filter(|reservation| reservation.account_id == account_id)
        .count() as i64
}

pub(crate) fn pool_routing_model_reservation_count(
    state: &AppState,
    account_id: i64,
    model: Option<&str>,
) -> i64 {
    let Some(model) = model.map(str::trim) else {
        return 0;
    };
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations
        .values()
        .filter(|reservation| {
            reservation.account_id == account_id
                && reservation
                    .model
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(model))
        })
        .count() as i64
}

pub(crate) fn pool_routing_model_reservation_is_at_capacity(
    state: &AppState,
    reservation_key: &str,
    account_id: i64,
    model: Option<&str>,
    model_concurrency_limit: Option<i64>,
) -> bool {
    let (Some(model), Some(limit)) = (
        model.map(str::trim).filter(|value| !value.is_empty()),
        model_concurrency_limit,
    ) else {
        return false;
    };
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let active = reservations
        .iter()
        .filter(|(key, reservation)| {
            key.as_str() != reservation_key
                && reservation.account_id == account_id
                && reservation
                    .model
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(model))
        })
        .count() as i64;
    active >= limit.max(1)
}

pub(crate) fn pool_routing_reservation_matches_model(
    state: &AppState,
    reservation_key: &str,
    account_id: i64,
    model: Option<&str>,
) -> bool {
    let Some(model) = model.map(str::trim) else {
        return false;
    };
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations
        .get(reservation_key)
        .is_some_and(|reservation| {
            reservation.account_id == account_id
                && reservation
                    .model
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case(model))
        })
}

pub(crate) fn pool_routing_reservation_snapshot(
    state: &AppState,
) -> PoolRoutingReservationSnapshot {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let mut snapshot = PoolRoutingReservationSnapshot::default();
    for reservation in reservations.values() {
        *snapshot
            .counts_by_account
            .entry(reservation.account_id)
            .or_default() += 1;
        if let Some(proxy_key) = reservation.proxy_key.as_deref() {
            snapshot.reserved_proxy_keys.insert(proxy_key.to_string());
            snapshot
                .proxy_keys_by_account
                .entry(reservation.account_id)
                .or_default()
                .insert(proxy_key.to_string());
        }
    }
    snapshot
}

pub(crate) fn reserve_pool_routing_account(
    state: &AppState,
    reservation_key: &str,
    account: &PoolResolvedAccount,
) {
    reserve_pool_routing_account_for_model(state, reservation_key, account, None);
}

pub(crate) fn reserve_pool_routing_account_for_model(
    state: &AppState,
    reservation_key: &str,
    account: &PoolResolvedAccount,
    model: Option<&str>,
) {
    let _ =
        try_reserve_pool_routing_account_for_model(state, reservation_key, account, model, None);
}

/// Atomically checks a model-route cap and records the request reservation.
///
/// The caller resolves the database-backed cap before entering this process-local
/// mutex. The capacity check and reservation insertion themselves must share one
/// lock so concurrent candidate selection cannot admit more than the configured
/// number of requests for the same account/model pair.
pub(crate) fn try_reserve_pool_routing_account_for_model(
    state: &AppState,
    reservation_key: &str,
    account: &PoolResolvedAccount,
    model: Option<&str>,
    model_concurrency_limit: Option<i64>,
) -> bool {
    let proxy_key = match &account.forward_proxy_scope {
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => Some(proxy_key.clone()),
        _ => None,
    };
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let model = model.map(str::trim).map(ToOwned::to_owned).or_else(|| {
        reservations
            .get(reservation_key)
            .and_then(|reservation| reservation.model.clone())
    });
    if account.routing_source == PoolRoutingSelectionSource::StickyReuse
        && proxy_key.is_none()
        && model.is_none()
    {
        return true;
    }
    if let (Some(limit), Some(model)) = (model_concurrency_limit, model.as_deref()) {
        let active = reservations
            .iter()
            .filter(|(key, reservation)| {
                key.as_str() != reservation_key
                    && reservation.account_id == account.account_id
                    && reservation
                        .model
                        .as_deref()
                        .is_some_and(|value| value.eq_ignore_ascii_case(model))
            })
            .count() as i64;
        if active >= limit.max(1) {
            return false;
        }
    }
    reservations.insert(
        reservation_key.to_string(),
        PoolRoutingReservation {
            account_id: account.account_id,
            model,
            proxy_key,
            created_at: Instant::now(),
        },
    );
    true
}

pub(crate) fn release_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    release_pool_routing_reservation_with_availability(state, reservation_key, true);
}

pub(crate) fn release_pool_routing_reservation_without_availability(
    state: &AppState,
    reservation_key: &str,
) {
    release_pool_routing_reservation_with_availability(state, reservation_key, false);
}

fn release_pool_routing_reservation_with_availability(
    state: &AppState,
    reservation_key: &str,
    publish_availability: bool,
) {
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let released = reservations.remove(reservation_key).is_some();
    drop(reservations);
    if released && publish_availability {
        state.pool_routing_availability.publish();
    }
}

pub(crate) async fn persist_pool_route_failure_then_release<T, E>(
    state: &AppState,
    reservation_key: &str,
    persist_failure: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    persist_pool_route_failure_then_release_with_guard(
        state,
        reservation_key,
        None,
        persist_failure,
    )
    .await
}

pub(crate) async fn persist_pool_route_failure_then_release_with_guard<T, E>(
    state: &AppState,
    reservation_key: &str,
    reservation_guard: Option<&mut PoolRoutingReservationDropGuard>,
    persist_failure: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let result = if let Some(guard) = reservation_guard {
        guard.fence_failure(persist_failure).await
    } else {
        persist_failure.await
    };
    match result {
        Ok(value) => {
            release_pool_routing_reservation(state, reservation_key);
            Ok(value)
        }
        Err(err) => {
            // The failed write did not fence this route. Release its slot so it cannot leak,
            // but do not wake waiters into an immediate unfenced retry.
            release_pool_routing_reservation_without_availability(state, reservation_key);
            Err(err)
        }
    }
}

pub(crate) async fn persist_pool_route_success_then_release<E>(
    state: &AppState,
    reservation_key: &str,
    persist_success: impl std::future::Future<Output = Result<bool, E>>,
) -> Result<(), E> {
    match persist_success.await {
        Ok(publish_availability) => {
            // Successful capacity release wakes waiters only while its account is
            // still selectable. A stale success keeps the newer failure fenced.
            release_pool_routing_reservation_with_availability(
                state,
                reservation_key,
                publish_availability,
            );
            Ok(())
        }
        Err(err) => {
            release_pool_routing_reservation_without_availability(state, reservation_key);
            Err(err)
        }
    }
}

pub(crate) fn publish_pool_routing_availability(state: &AppState) {
    state.pool_routing_availability.publish();
}

pub(crate) fn consume_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    release_pool_routing_reservation(state, reservation_key);
}
