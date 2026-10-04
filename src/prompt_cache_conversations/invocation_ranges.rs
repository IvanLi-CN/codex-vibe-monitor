//! Durable reservation authority shared by conversation and hourly invocation owners.

use super::*;
use tokio::sync::Notify;

pub(crate) mod lifecycle;

const RANGE_SIZE: u32 = 64;
const WAIT_BUDGET: Duration = Duration::from_millis(100);
const MIN_CAPACITY: usize = 128;
const MAX_CAPACITY: usize = 4096;
const ACTIVITY_WINDOW_MS: i64 = 48 * 3600 * 1000;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) enum Owner {
    Conversation(String),
    Hour(i64),
}

impl Owner {
    fn from_key(key: Option<&str>) -> Self {
        match normalize_prompt_cache_key(key) {
            Some(key) => Self::Conversation(key.to_owned()),
            None => Self::Hour(Utc::now().timestamp().div_euclid(3600)),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Conversation(_) => "conversation",
            Self::Hour(_) => "hour",
        }
    }

    fn hour(&self) -> Option<i64> {
        match self {
            Self::Hour(hour) => Some(*hour),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Range {
    next: u32,
    end: u32,
}

impl Range {
    fn remaining(self) -> u32 {
        self.end - self.next
    }
}

#[derive(Debug)]
struct Entry {
    generation: u64,
    prefix: Option<String>,
    current: Range,
    standby: Option<Range>,
    ceiling: i64,
    issued_floor: i64,
    operation: bool,
    retiring: bool,
    failure: Option<String>,
    last_used: i64,
    notify: Arc<Notify>,
    namespace: Option<lifecycle::PrefixGuard>,
}

impl Entry {
    fn initializing(generation: u64) -> Self {
        Self {
            generation,
            prefix: None,
            current: Range { next: 0, end: 0 },
            standby: None,
            ceiling: -1,
            issued_floor: -1,
            operation: true,
            retiring: false,
            failure: None,
            last_used: Utc::now().timestamp_millis(),
            notify: Arc::new(Notify::new()),
            namespace: None,
        }
    }

    fn can_refill(&self, remaining: u32) -> bool {
        self.prefix.is_some()
            && !self.operation
            && !self.retiring
            && self.standby.is_none()
            && self.current.remaining() < remaining
            && self.ceiling < i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY - 1)
    }
}

#[derive(Debug)]
struct Memory {
    entries: HashMap<Owner, Entry>,
    generation: u64,
    batch_running: bool,
    target: usize,
    leases: HashMap<String, usize>,
    allocations: HashMap<Owner, usize>,
    active_ids: HashSet<String>,
    active_prefixes: HashMap<String, usize>,
    hourly_cleanup_cursor: Option<i64>,
    activity: HashMap<String, i64>,
    activity_order: std::collections::BTreeSet<(i64, String)>,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            batch_running: false,
            target: MIN_CAPACITY,
            leases: HashMap::new(),
            allocations: HashMap::new(),
            active_ids: HashSet::new(),
            active_prefixes: HashMap::new(),
            hourly_cleanup_cursor: None,
            activity: HashMap::new(),
            activity_order: std::collections::BTreeSet::new(),
        }
    }
}

impl Memory {
    fn occupancy(&self) -> usize {
        self.entries
            .keys()
            .filter(|owner| matches!(owner, Owner::Conversation(_)))
            .count()
    }

    fn observe(&mut self, prefix: String, timestamp: i64) {
        if let Some(previous) = self.activity.get(&prefix).copied() {
            if previous >= timestamp {
                return;
            }
            self.activity_order.remove(&(previous, prefix.clone()));
        }
        self.activity.insert(prefix.clone(), timestamp);
        self.activity_order.insert((timestamp, prefix));
        while self.activity.len() > MAX_CAPACITY {
            let (_, oldest) = self
                .activity_order
                .pop_first()
                .expect("bounded activity order");
            self.activity.remove(&oldest);
        }
    }

    fn resize(&mut self, now_ms: i64, source: &'static str) {
        while self
            .activity_order
            .first()
            .is_some_and(|(time, _)| *time < now_ms - ACTIVITY_WINDOW_MS)
        {
            let (_, prefix) = self.activity_order.pop_first().unwrap();
            self.activity.remove(&prefix);
        }
        let previous = self.target;
        self.target = self.activity.len().clamp(MIN_CAPACITY, MAX_CAPACITY);
        info!(
            source,
            cutoff_ms = now_ms - ACTIVITY_WINDOW_MS,
            capped = self.activity.len() == MAX_CAPACITY,
            activity_count_lower_bound = self.activity.len(),
            old_target = previous,
            new_target = self.target,
            occupancy = self.occupancy(),
            deferred_shrink = self.occupancy().saturating_sub(self.target),
            "invocation cache capacity estimated from memory activity"
        );
    }

    fn retire_candidate(&mut self) -> Option<(Reservation, i64)> {
        // A retirement can wait for SQLite worker drain after its 100 ms budget.
        // Keep that work bounded instead of starting one return per callback.
        if self.entries.values().any(|entry| entry.retiring) {
            return None;
        }
        let owner = self
            .entries
            .iter()
            .filter(|(owner, entry)| {
                matches!(owner, Owner::Conversation(key) if !self.leases.contains_key(key))
                    && !self.allocations.contains_key(*owner)
                    && !entry
                        .prefix
                        .as_deref()
                        .is_some_and(lifecycle::pending_prefix)
                    && !entry.operation
                    && !entry.retiring
            })
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(owner, _)| owner.clone())?;
        let entry = self.entries.get_mut(&owner).unwrap();
        entry.retiring = true;
        Some((
            Reservation {
                owner,
                prefix: entry.prefix.clone().unwrap_or_default(),
                generation: entry.generation,
                floor: entry.issued_floor,
            },
            entry.ceiling,
        ))
    }
}

/// Memory is the issuance authority; only committed SQLite ranges may enter it.
/// No memory guard survives an await, including on initialization and refill.
#[derive(Debug, Default)]
pub(crate) struct InvocationRangeManager {
    memory: std::sync::Mutex<Memory>,
    admission: Notify,
    pool: std::sync::OnceLock<Pool<Sqlite>>,
    sizing_started: std::sync::atomic::AtomicBool,
}

#[derive(Debug, Clone)]
struct Reservation {
    owner: Owner,
    prefix: String,
    generation: u64,
    floor: i64,
}

struct AllocationGuard {
    manager: Arc<InvocationRangeManager>,
    owner: Owner,
}

impl Drop for AllocationGuard {
    fn drop(&mut self) {
        {
            let mut memory = self.manager.memory.lock().expect("invocation range memory");
            if let Some(count) = memory.allocations.get_mut(&self.owner) {
                *count -= 1;
                if *count == 0 {
                    memory.allocations.remove(&self.owner);
                }
            }
        }
        self.manager.admission.notify_waiters();
        self.manager.shrink();
    }
}

impl InvocationRangeManager {
    pub(crate) fn occupancy(&self) -> usize {
        self.memory
            .lock()
            .expect("invocation range memory")
            .occupancy()
    }
    pub(crate) fn reconcile_persistence(self: &Arc<Self>, id: &str) {
        {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if memory.active_ids.remove(id) {
                let prefix = &id[..6];
                if let Some(count) = memory.active_prefixes.get_mut(prefix) {
                    *count -= 1;
                    if *count == 0 {
                        memory.active_prefixes.remove(prefix);
                    }
                }
            }
        }
        self.admission.notify_waiters();
        self.shrink();
    }

    #[cfg(test)]
    pub(crate) fn test_observe_activity(&self, prefix: &str, timestamp: i64) {
        self.memory
            .lock()
            .unwrap()
            .observe(prefix.to_owned(), timestamp);
    }

    #[cfg(test)]
    pub(crate) fn test_activity_time(&self, prefix: &str) -> Option<i64> {
        self.memory.lock().unwrap().activity.get(prefix).copied()
    }

    #[cfg(test)]
    pub(crate) async fn test_allocate_hour(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        hour: i64,
    ) -> Result<String> {
        self.pool.get_or_init(|| pool.clone());
        self.allocate_owner(pool, Owner::Hour(hour), true).await
    }

    #[cfg(test)]
    pub(crate) fn test_resize(&self, timestamp: i64) -> (usize, usize, usize) {
        let mut memory = self.memory.lock().unwrap();
        memory.resize(timestamp, "controlled_clock");
        (memory.target, memory.occupancy(), memory.activity.len())
    }

    #[cfg(test)]
    pub(crate) fn test_retire(self: &Arc<Self>, pool: &Pool<Sqlite>, key: &str) {
        let (reservation, ceiling) = {
            let mut memory = self.memory.lock().unwrap();
            assert!(!memory.leases.contains_key(key));
            let owner = Owner::Conversation(key.to_owned());
            let entry = memory.entries.get_mut(&owner).unwrap();
            assert!(!entry.operation && !entry.retiring);
            entry.retiring = true;
            (
                Reservation {
                    owner,
                    prefix: entry.prefix.clone().unwrap(),
                    generation: entry.generation,
                    floor: entry.issued_floor,
                },
                entry.ceiling,
            )
        };
        self.spawn_return(pool, reservation, ceiling);
    }

    pub(crate) fn retain_key(&self, key: &str) {
        *self
            .memory
            .lock()
            .expect("invocation range memory")
            .leases
            .entry(key.to_owned())
            .or_default() += 1;
    }

    pub(crate) fn release_key(self: &Arc<Self>, key: &str) {
        {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if let Some(count) = memory.leases.get_mut(key) {
                *count -= 1;
                if *count == 0 {
                    memory.leases.remove(key);
                }
            }
        }
        self.admission.notify_waiters();
        self.shrink();
    }

    pub(crate) fn start_sizing(self: &Arc<Self>, pool: &Pool<Sqlite>, shutdown: CancellationToken) {
        if self.sizing_started.swap(true, Ordering::SeqCst) {
            return;
        }
        self.pool.get_or_init(|| pool.clone());
        let manager = self.clone();
        let pool = pool.clone();
        tokio::spawn(async move {
            let now = Utc::now();
            let cutoff = now - chrono::Duration::hours(48);
            // The master preserves the existing Shanghai-local invocation timestamp
            // representation. Convert the captured UTC instant, without SQL functions
            // on the indexed column, so the bounded covering range scan stays usable.
            let query_cutoff = format_naive(cutoff.with_timezone(&Shanghai).naive_local());
            let started = Instant::now();
            let seed = tokio::time::timeout(Duration::from_secs(2), sqlx::query_as::<_, (String, String)>(
                "SELECT conversation_id,last_invocation_at FROM prompt_cache_conversations INDEXED BY idx_prompt_cache_conversations_last_invocation WHERE last_invocation_at>=?1 ORDER BY last_invocation_at DESC,conversation_id LIMIT 4096"
            ).bind(query_cutoff).fetch_all(&pool)).await;
            match seed {
                Ok(Ok(rows)) => {
                    let mut memory = manager.memory.lock().expect("invocation range memory");
                    for (prefix, timestamp) in &rows {
                        if let Some(timestamp) = activity_timestamp_ms(timestamp) {
                            memory.observe(prefix.clone(), timestamp);
                        }
                    }
                    info!(source = "database_seed", cutoff_utc = %cutoff, row_count = rows.len(), capped = rows.len() == MAX_CAPACITY, elapsed_ms = started.elapsed().as_millis(), "invocation cache activity seed merged");
                    memory.resize(Utc::now().timestamp_millis(), "database_seed");
                }
                result => {
                    warn!(source = "database_seed", cutoff_utc = %cutoff, elapsed_ms = started.elapsed().as_millis(), error = ?result, "invocation activity seed unavailable; using memory estimate without retry")
                }
            }
            manager.shrink();
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(3600)) => {
                        manager.memory.lock().expect("invocation range memory").resize(Utc::now().timestamp_millis(), "hourly_memory");
                        manager.shrink();
                    }
                }
            }
        });
    }

    async fn admit(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        owner: &Owner,
        deadline: tokio::time::Instant,
    ) -> Result<Option<u64>> {
        loop {
            let notified = self.admission.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let retirement = {
                let mut memory = self.memory.lock().expect("invocation range memory");
                if memory
                    .entries
                    .get(owner)
                    .is_some_and(|entry| !entry.retiring)
                {
                    return Ok(None);
                }
                if memory
                    .entries
                    .get(owner)
                    .is_some_and(|entry| entry.retiring)
                {
                    None
                } else {
                    let occupancy = memory.occupancy();
                    if matches!(owner, Owner::Hour(_)) || occupancy < MAX_CAPACITY {
                        if occupancy >= memory.target && matches!(owner, Owner::Conversation(_)) {
                            debug!(
                                target = memory.target,
                                occupancy,
                                maximum = MAX_CAPACITY,
                                "invocation cache grows until asynchronous retirement catches up"
                            );
                        }
                        memory.generation += 1;
                        let generation = memory.generation;
                        memory
                            .entries
                            .insert(owner.clone(), Entry::initializing(generation));
                        return Ok(Some(generation));
                    }
                    memory.retire_candidate()
                }
            };
            if let Some((reservation, ceiling)) = retirement {
                self.spawn_return(pool, reservation, ceiling);
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                let memory = self.memory.lock().expect("invocation range memory");
                warn!(
                    owner_type = owner.kind(),
                    target = memory.target,
                    occupancy = memory.occupancy(),
                    maximum = MAX_CAPACITY,
                    budget_ms = 100,
                    "invocation cache admission timed out"
                );
                bail!("invocation cache admission timed out after 100ms");
            }
        }
    }

    fn shrink(self: &Arc<Self>) {
        let Some(pool) = self.pool.get() else {
            return;
        };
        let retirement = {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if memory
                .entries
                .iter()
                .filter(|(owner, entry)| matches!(owner, Owner::Conversation(_)) && !entry.retiring)
                .count()
                > memory.target
            {
                memory.retire_candidate()
            } else {
                None
            }
        };
        if let Some((reservation, ceiling)) = retirement {
            self.spawn_return(pool, reservation, ceiling);
        }
    }

    fn spawn_return(self: &Arc<Self>, pool: &Pool<Sqlite>, reservation: Reservation, ceiling: i64) {
        let manager = self.clone();
        let pool = pool.clone();
        // The clock includes queueing the worker, admission and connection acquisition.
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + WAIT_BUDGET;
        tokio::spawn(async move {
            let mut connection = None;
            let mut permit = None;
            let result = tokio::time::timeout_at(deadline, async {
                if reservation.prefix.is_empty() { return Ok(()); }
                permit = Some(crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                    .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy).await);
                connection = Some(pool.acquire().await?);
                let connection = connection.as_mut().unwrap();
                return_tail(connection, &reservation, ceiling).await
            }).await;
            let confirmed = matches!(&result, Ok(Ok(())));
            if confirmed {
                info!(owner_type = reservation.owner.kind(), prefix = %reservation.prefix, generation = reservation.generation, expected_ceiling = ceiling, return_floor = reservation.floor, attempted_return_count = ceiling - reservation.floor, elapsed_ms = started.elapsed().as_millis(), "unused invocation reservation tail returned");
            } else {
                warn!(owner_type = reservation.owner.kind(), prefix = %reservation.prefix, generation = reservation.generation, expected_ceiling = ceiling, return_floor = reservation.floor, attempted_return_count = ceiling - reservation.floor, elapsed_ms = started.elapsed().as_millis(), error = ?result, "invocation tail return unconfirmed; discarding issuance rights and retaining fence until connection closes");
            }
            // A timed-out SQLite future may still be executing in its worker. Closing
            // this owned connection drains that worker before replacement is admitted.
            if let Some(connection) = connection {
                if confirmed {
                    drop(connection);
                } else if let Err(error) = connection.close().await {
                    warn!(prefix = %reservation.prefix, generation = reservation.generation, error = %error, "retired invocation connection close failed");
                }
            }
            drop(permit);
            let mut memory = manager.memory.lock().expect("invocation range memory");
            if memory
                .entries
                .get(&reservation.owner)
                .is_some_and(|entry| entry.generation == reservation.generation && entry.retiring)
                && let Some(entry) = memory.entries.remove(&reservation.owner)
            {
                entry.notify.notify_waiters();
            }
            manager.admission.notify_waiters();
        });
    }

    pub(crate) async fn allocate(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        key: Option<&str>,
    ) -> Result<String> {
        self.pool.get_or_init(|| pool.clone());
        self.allocate_owner(pool, Owner::from_key(key), false).await
    }

    pub(crate) async fn allocate_leased(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        key: Option<&str>,
    ) -> Result<String> {
        self.pool.get_or_init(|| pool.clone());
        self.allocate_owner(pool, Owner::from_key(key), true).await
    }

    async fn allocate_owner(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        owner: Owner,
        leased: bool,
    ) -> Result<String> {
        *self
            .memory
            .lock()
            .expect("invocation range memory")
            .allocations
            .entry(owner.clone())
            .or_default() += 1;
        let _guard = AllocationGuard {
            manager: self.clone(),
            owner: owner.clone(),
        };
        let deadline = tokio::time::Instant::now() + WAIT_BUDGET;
        loop {
            let initialization = self.admit(pool, &owner, deadline).await?;
            let (notify, initialization, issued, trigger) = {
                let mut memory = self.memory.lock().expect("invocation range memory");
                let Some(entry) = memory.entries.get_mut(&owner) else {
                    continue;
                };
                if let Some(failure) = entry.failure.clone() {
                    let notify = entry.notify.clone();
                    memory.entries.remove(&owner);
                    notify.notify_waiters();
                    self.admission.notify_waiters();
                    bail!("invocation range allocation failed: {failure}");
                }
                let issued = if !entry.retiring && entry.prefix.is_some() {
                    if entry.current.remaining() == 0
                        && let Some(standby) = entry.standby.take()
                    {
                        entry.current = standby;
                    }
                    if entry.current.remaining() > 0 {
                        let sequence = entry.current.next;
                        entry.current.next += 1;
                        entry.issued_floor = i64::from(sequence);
                        entry.last_used = Utc::now().timestamp_millis();
                        Some(format!(
                            "{}{}",
                            entry.prefix.as_deref().unwrap(),
                            encode_prompt_cache_conversation_sequence(sequence)?
                        ))
                    } else if !entry.operation
                        && entry.ceiling
                            == i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY - 1)
                    {
                        error!(owner_type = owner.kind(), utc_hour = owner.hour(), prefix = ?entry.prefix, generation = entry.generation, "invocation sequence exhausted");
                        bail!(
                            "prompt-cache conversation invoke sequence overflow: capacity={}",
                            PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY
                        );
                    } else {
                        None
                    }
                } else {
                    None
                };
                let notify = entry.notify.clone();
                let trigger = entry.can_refill(32);
                if leased
                    && matches!(owner, Owner::Hour(_))
                    && let Some(id) = &issued
                    && memory.active_ids.insert(id.clone())
                {
                    *memory
                        .active_prefixes
                        .entry(id[..6].to_owned())
                        .or_default() += 1;
                }
                (notify, initialization, issued, trigger)
            };
            // Register notification before dispatch so a fast commit cannot be missed.
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(generation) = initialization {
                let manager = self.clone();
                let pool = pool.clone();
                let owner = owner.clone();
                tokio::spawn(async move {
                    manager.initialize(&pool, owner, generation).await;
                });
            }
            if trigger {
                self.start_refill(pool);
            }
            if let Some(invoke_id) = issued {
                if matches!(owner, Owner::Conversation(_)) {
                    self.memory
                        .lock()
                        .expect("invocation range memory")
                        .observe(invoke_id[..6].to_owned(), Utc::now().timestamp_millis());
                }
                debug!(owner_type = owner.kind(), utc_hour = owner.hour(), invoke_id = %invoke_id, "allocated proxy invoke id from committed memory range");
                return Ok(invoke_id);
            }
            // Check again after enabling notification: completion can precede enable().
            let ready = {
                let memory = self.memory.lock().expect("invocation range memory");
                memory.entries.get(&owner).is_some_and(|entry| {
                    !entry.retiring
                        && (entry.failure.is_some()
                            || entry.current.remaining() > 0
                            || entry.standby.is_some())
                })
            };
            if ready {
                continue;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                warn!(
                    owner_type = owner.kind(),
                    utc_hour = owner.hour(),
                    budget_ms = 100,
                    "invocation range wait timed out"
                );
                bail!("invocation range allocation timed out after 100ms");
            }
        }
    }

    async fn initialize(self: &Arc<Self>, pool: &Pool<Sqlite>, owner: Owner, generation: u64) {
        let started = Instant::now();
        let result = self.initialize_committed(pool, &owner, generation).await;
        let mut memory = self.memory.lock().expect("invocation range memory");
        let Some(entry) = memory
            .entries
            .get_mut(&owner)
            .filter(|entry| entry.generation == generation)
        else {
            return;
        };
        entry.operation = false;
        match result {
            Ok((prefix, range)) => {
                entry.namespace = Some(lifecycle::PrefixGuard::new(&prefix));
                entry.prefix = Some(prefix.clone());
                entry.ceiling = i64::from(range.end) - 1;
                entry.issued_floor = i64::from(range.next) - 1;
                entry.current = range;
                info!(owner_type = owner.kind(), utc_hour = owner.hour(), prefix = %prefix, generation, range_start = range.next, range_end = range.end, elapsed_ms = started.elapsed().as_millis(), "invocation initial committed range published");
            }
            Err(error) => {
                warn!(owner_type = owner.kind(), utc_hour = owner.hour(), generation, error = %error, elapsed_ms = started.elapsed().as_millis(), "invocation range initialization failed");
                entry.failure = Some(error.to_string());
            }
        }
        entry.notify.notify_waiters();
        drop(memory);
        self.shrink();
    }

    async fn initialize_committed(
        &self,
        pool: &Pool<Sqlite>,
        owner: &Owner,
        generation: u64,
    ) -> Result<(String, Range)> {
        let _permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
            .await;
        let (prefix, floor) = match owner {
            Owner::Conversation(key) => {
                let identity = load_or_create_prompt_cache_conversation_identity(pool, key).await?;
                (
                    identity.conversation_id,
                    i64::from(identity.next_sequence) - 1,
                )
            }
            Owner::Hour(hour) => {
                let prefix = load_or_create_hour(pool, *hour).await?;
                let floor = max_live_sequence_for_conversation(pool, &prefix)
                    .await?
                    .map(i64::from)
                    .unwrap_or(-1);
                (prefix, floor)
            }
        };
        let floor = floor.max(lifecycle::pending_floor(&prefix));
        let reservation = Reservation {
            owner: owner.clone(),
            prefix: prefix.clone(),
            generation,
            floor,
        };
        let mut tx = pool.begin().await?;
        let range = reserve_on_connection(tx.as_mut(), &reservation).await?;
        tx.commit().await?;
        Ok((prefix, range))
    }

    fn start_refill(self: &Arc<Self>, pool: &Pool<Sqlite>) {
        let reservations = {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if memory.batch_running {
                return;
            }
            let current_hour = Utc::now().timestamp().div_euclid(3600);
            let reservations = memory
                .entries
                .iter_mut()
                .filter_map(|(owner, entry)| {
                    if owner.hour().is_some_and(|hour| hour != current_hour)
                        || !entry.can_refill(48)
                    {
                        return None;
                    }
                    entry.operation = true;
                    Some(Reservation {
                        owner: owner.clone(),
                        prefix: entry.prefix.clone().unwrap(),
                        generation: entry.generation,
                        floor: entry.ceiling,
                    })
                })
                .collect::<Vec<_>>();
            if reservations.is_empty() {
                return;
            }
            memory.batch_running = true;
            reservations
        };
        let manager = self.clone();
        let pool = pool.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let result = reserve_batch(&pool, &reservations).await;
            {
                let mut memory = manager.memory.lock().expect("invocation range memory");
                memory.batch_running = false;
                for (index, reservation) in reservations.iter().enumerate() {
                    let Some(entry) = memory.entries.get_mut(&reservation.owner).filter(|entry| {
                        entry.generation == reservation.generation && !entry.retiring
                    }) else {
                        continue;
                    };
                    entry.operation = false;
                    match &result {
                        Ok(ranges) => {
                            let range = ranges[index];
                            entry.ceiling = i64::from(range.end) - 1;
                            entry.standby = Some(range);
                            info!(owner_type = reservation.owner.kind(), utc_hour = reservation.owner.hour(), prefix = %reservation.prefix, generation = reservation.generation, range_start = range.next, range_end = range.end, batch_size = reservations.len(), elapsed_ms = started.elapsed().as_millis(), "invocation committed standby range published");
                        }
                        Err(error) => {
                            if entry.current.remaining() == 0 {
                                entry.failure = Some(error.to_string());
                            }
                            warn!(owner_type = reservation.owner.kind(), prefix = %reservation.prefix, generation = reservation.generation, error = %error, "invocation batch refill failed; retaining committed remainder");
                        }
                    }
                    entry.notify.notify_waiters();
                }
                // A new trigger may have arrived while this batch was in flight.
                // Wake those owners too; they join the next shared batch, never SQL fallback.
                for entry in memory.entries.values() {
                    entry.notify.notify_waiters();
                }
            }
            manager.shrink();
        });
    }
}

async fn reserve_batch(pool: &Pool<Sqlite>, reservations: &[Reservation]) -> Result<Vec<Range>> {
    let _permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let mut tx = pool.begin().await?;
    let mut ranges = Vec::with_capacity(reservations.len());
    for reservation in reservations {
        ranges.push(reserve_on_connection(tx.as_mut(), reservation).await?);
    }
    tx.commit().await?;
    Ok(ranges)
}

async fn reserve_on_connection(
    connection: &mut SqliteConnection,
    reservation: &Reservation,
) -> Result<Range> {
    let (table, column, identity) = match &reservation.owner {
        Owner::Conversation(key) => (
            "prompt_cache_conversations",
            "prompt_cache_key",
            key.clone(),
        ),
        Owner::Hour(hour) => ("hourly_invoke_prefixes", "utc_hour", hour.to_string()),
    };
    let prefix_column = if matches!(reservation.owner, Owner::Hour(_)) {
        "prefix"
    } else {
        "conversation_id"
    };
    let ceiling = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT last_invoke_sequence FROM {table} WHERE {column}=?1 AND {prefix_column}=?2"
    ))
    .bind(&identity)
    .bind(&reservation.prefix)
    .fetch_one(&mut *connection)
    .await?;
    let start = ceiling.max(reservation.floor) + 1;
    let end =
        (start + i64::from(RANGE_SIZE)).min(i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY));
    if start >= end {
        bail!(
            "invocation sequence overflow: capacity={}",
            PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY
        );
    }
    let updated = sqlx::query(&format!("UPDATE {table} SET last_invoke_sequence=?1,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE {column}=?2 AND {prefix_column}=?3 AND last_invoke_sequence=?4"))
        .bind(end - 1).bind(identity).bind(&reservation.prefix).bind(ceiling).execute(&mut *connection).await?;
    if updated.rows_affected() != 1 {
        bail!("invocation reservation owner changed");
    }
    debug!(owner_type = reservation.owner.kind(), utc_hour = reservation.owner.hour(), prefix = %reservation.prefix, generation = reservation.generation, range_start = start, range_end = end, "invocation range reserved; awaiting commit");
    Ok(Range {
        next: start as u32,
        end: end as u32,
    })
}

async fn load_or_create_hour(pool: &Pool<Sqlite>, hour: i64) -> Result<String> {
    let _namespace = PROMPT_CACHE_UNBOUND_PREFIX_NAMESPACE.lock().await;
    if let Some(prefix) = sqlx::query_scalar::<_, String>(
        "SELECT prefix FROM hourly_invoke_prefixes WHERE utc_hour=?1",
    )
    .bind(hour)
    .fetch_optional(pool)
    .await?
    {
        info!(utc_hour = hour, prefix = %prefix, "hourly invocation prefix recovered");
        return Ok(prefix);
    }
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let prefix = generate_prompt_cache_conversation_id();
        if prompt_cache_conversation_id_candidate_conflicts(pool, &prefix).await? {
            continue;
        }
        sqlx::query("INSERT INTO hourly_invoke_prefixes (utc_hour,prefix) VALUES (?1,?2)")
            .bind(hour)
            .bind(&prefix)
            .execute(pool)
            .await?;
        info!(utc_hour = hour, prefix = %prefix, attempt, "hourly invocation prefix created");
        return Ok(prefix);
    }
    bail!("failed to allocate hourly invoke prefix after 5 attempts")
}

fn activity_timestamp_ms(value: &str) -> Option<i64> {
    if let Ok(timestamp) = DateTime::parse_from_rfc3339(value) {
        return Some(timestamp.timestamp_millis());
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f").ok()?;
    Shanghai
        .from_local_datetime(&naive)
        .single()
        .map(|timestamp| timestamp.timestamp_millis())
}

async fn return_tail(
    connection: &mut SqliteConnection,
    reservation: &Reservation,
    ceiling: i64,
) -> Result<()> {
    let (table, column, identity, prefix_column) = match &reservation.owner {
        Owner::Conversation(key) => (
            "prompt_cache_conversations",
            "prompt_cache_key",
            key.clone(),
            "conversation_id",
        ),
        Owner::Hour(hour) => (
            "hourly_invoke_prefixes",
            "utc_hour",
            hour.to_string(),
            "prefix",
        ),
    };
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *connection)
        .await?;
    let updated = sqlx::query(&format!("UPDATE {table} SET last_invoke_sequence=?1,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE {column}=?2 AND {prefix_column}=?3 AND last_invoke_sequence=?4"))
        .bind(reservation.floor).bind(identity).bind(&reservation.prefix).bind(ceiling).execute(&mut *connection).await?;
    if updated.rows_affected() != 1 {
        bail!("invocation tail return condition did not match");
    }
    sqlx::query("COMMIT").execute(connection).await?;
    Ok(())
}

const MIGRATION: &str = "invocation_range_ownership_v1";

pub(crate) async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let alphabet = PROXY_INVOKE_ID_ALPHABET.iter().collect::<String>();
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS hourly_invoke_prefixes (\
            utc_hour INTEGER PRIMARY KEY,\
            prefix TEXT NOT NULL UNIQUE,\
            last_invoke_sequence INTEGER NOT NULL DEFAULT -1,\
            created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')),\
            updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')),\
            CHECK (length(prefix)=6 AND prefix NOT GLOB '*[^{alphabet}]*'),\
            CHECK (last_invoke_sequence BETWEEN -1 AND 923520)\
        )"
    ))
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT OR IGNORE INTO schema_refresh_migrations (migration_name) VALUES (?1)")
        .bind(MIGRATION)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    debug!(migration = MIGRATION, "invocation range schema ready");
    Ok(())
}
