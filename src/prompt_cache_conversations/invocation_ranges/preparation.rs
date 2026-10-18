//! Best-effort preparation never queues ahead of actual missing-range demand.
use super::*;

pub(super) const STARTUP_OWNER_LIMIT: usize = 128;

pub(super) enum PreparationOutcome {
    Ready,
    Missing,
    Deferred,
}

const RETRY_INITIAL_DELAY: Duration = Duration::from_secs(1);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct RetryKey {
    owner: Owner,
    generation: u64,
    hour: i64,
}

struct RetryItem {
    key: RetryKey,
    attempts: u8,
    retry_at: std::time::Instant,
}

#[derive(Default)]
struct RetryQueue {
    pending: std::collections::VecDeque<RetryItem>,
    queued: std::collections::HashSet<RetryKey>,
    active: std::collections::HashSet<RetryKey>,
}

impl RetryQueue {
    fn len(&self) -> usize {
        self.pending.len() + self.active.len()
    }

    fn enqueue(&mut self, item: RetryItem) -> bool {
        if self.queued.contains(&item.key) || self.active.contains(&item.key) {
            return true;
        }
        if self.len() >= STARTUP_OWNER_LIMIT {
            return false;
        }
        self.queued.insert(item.key.clone());
        self.pending.push_back(item);
        true
    }

    fn take_due(&mut self, now: std::time::Instant) -> Option<RetryItem> {
        let index = self.pending.iter().position(|item| item.retry_at <= now)?;
        let item = self.pending.remove(index)?;
        self.queued.remove(&item.key);
        self.active.insert(item.key.clone());
        Some(item)
    }

    fn complete(&mut self, key: &RetryKey) {
        self.active.remove(key);
    }

    fn defer(&mut self, previous: &RetryKey, item: RetryItem) -> bool {
        self.active.remove(previous);
        self.enqueue(item)
    }

    fn generation_for(&self, owner: &Owner, hour: i64) -> Option<u64> {
        self.pending
            .iter()
            .find(|item| item.key.owner == *owner && item.key.hour == hour)
            .map(|item| item.key.generation)
            .or_else(|| {
                self.active
                    .iter()
                    .find(|key| key.owner == *owner && key.hour == hour)
                    .map(|key| key.generation)
            })
    }

    #[cfg(test)]
    fn make_due(&mut self) {
        let now = std::time::Instant::now();
        for item in &mut self.pending {
            item.retry_at = now;
        }
    }
}

struct ManagedRetryQueue {
    manager: std::sync::Weak<InvocationRangeManager>,
    queue: RetryQueue,
}

static RETRY_QUEUES: std::sync::OnceLock<std::sync::Mutex<Vec<ManagedRetryQueue>>> =
    std::sync::OnceLock::new();

fn with_retry_queue<T>(
    manager: &Arc<InvocationRangeManager>,
    operation: impl FnOnce(&mut RetryQueue) -> T,
) -> T {
    let mut managers = RETRY_QUEUES
        .get_or_init(Default::default)
        .lock()
        .expect("invocation preparation retry queues");
    managers.retain(|managed| managed.manager.strong_count() > 0);
    let manager_ref = Arc::downgrade(manager);
    let index = managers
        .iter()
        .position(|managed| std::sync::Weak::ptr_eq(&managed.manager, &manager_ref))
        .unwrap_or_else(|| {
            managers.push(ManagedRetryQueue {
                manager: manager_ref,
                queue: RetryQueue::default(),
            });
            managers.len() - 1
        });
    operation(&mut managers[index].queue)
}

fn hours(now: DateTime<Utc>) -> impl Iterator<Item = Owner> {
    let hour = now.timestamp().div_euclid(3600);
    let next = (now.timestamp().rem_euclid(3600) >= 3540).then_some(Owner::Hour(hour + 1));
    std::iter::once(Owner::Hour(hour)).chain(next)
}

impl InvocationRangeManager {
    fn enqueue_retry_owner(self: &Arc<Self>, owner: Owner, hour: i64) {
        let owner_type = owner.kind();
        let (accepted, generation) = with_retry_queue(self, |queue| {
            let existing_generation = {
                self.memory
                    .lock()
                    .expect("invocation range memory")
                    .entries
                    .get(&owner)
                    .map(|entry| entry.generation)
            };
            let generation = existing_generation
                .or_else(|| queue.generation_for(&owner, hour))
                .unwrap_or_else(|| {
                    let mut memory = self.memory.lock().expect("invocation range memory");
                    memory.generation += 1;
                    memory.generation
                });
            let accepted = queue.enqueue(RetryItem {
                key: RetryKey {
                    owner,
                    generation,
                    hour,
                },
                attempts: 0,
                retry_at: std::time::Instant::now(),
            });
            (accepted, generation)
        });
        if !accepted {
            record_recovery(RecoveryDiagnostic::RangePrewarmDeferred);
            warn!(
                owner_type,
                utc_hour = hour,
                generation,
                queue_limit = STARTUP_OWNER_LIMIT,
                reason = "queue_full",
                "speculative invocation range owner was not retained"
            );
        }
    }

    fn retry_generation(self: &Arc<Self>, owner: &Owner, fallback: u64) -> u64 {
        self.memory
            .lock()
            .expect("invocation range memory")
            .entries
            .get(owner)
            .map(|entry| entry.generation)
            .unwrap_or(fallback)
    }

    pub(super) async fn prepare_turn(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        now: DateTime<Utc>,
        owners: &mut std::collections::VecDeque<Owner>,
        shutdown: &CancellationToken,
    ) {
        self.prune_prepared_hours(now.timestamp().div_euclid(3600));
        for owner in hours(now) {
            if shutdown.is_cancelled() {
                return;
            }
            self.prepare_owner(pool, owner).await;
        }
        let current_hour = now.timestamp().div_euclid(3600);
        while let Some(owner) = owners.pop_front() {
            self.enqueue_retry_owner(owner, current_hour);
        }
        for _ in 0..MAX_REFILL_BATCH_SIZE {
            if shutdown.is_cancelled() {
                break;
            }
            let Some(item) =
                with_retry_queue(self, |queue| queue.take_due(std::time::Instant::now()))
            else {
                break;
            };
            if shutdown.is_cancelled() {
                with_retry_queue(self, |queue| queue.complete(&item.key));
                break;
            }
            let generation = self.retry_generation(&item.key.owner, item.key.generation);
            let outcome = self
                .prepare_owner_generation(pool, item.key.owner.clone(), generation)
                .await;
            if matches!(outcome, PreparationOutcome::Deferred) {
                let attempts = item.attempts.saturating_add(1);
                let shift = u32::from(attempts.saturating_sub(1).min(6));
                let delay = Duration::from_secs(1_u64 << shift).min(RETRY_MAX_DELAY);
                let generation = self.retry_generation(&item.key.owner, generation);
                let retry = RetryItem {
                    key: RetryKey {
                        owner: item.key.owner.clone(),
                        generation,
                        hour: item.key.hour,
                    },
                    attempts,
                    retry_at: std::time::Instant::now() + delay.max(RETRY_INITIAL_DELAY),
                };
                let retained = with_retry_queue(self, |queue| queue.defer(&item.key, retry));
                if !retained {
                    warn!(
                        owner_type = item.key.owner.kind(),
                        utc_hour = item.key.hour,
                        generation,
                        attempts,
                        queue_limit = STARTUP_OWNER_LIMIT,
                        reason = "retry_queue_full",
                        "deferred invocation range retry was dropped"
                    );
                }
            } else {
                with_retry_queue(self, |queue| queue.complete(&item.key));
            }
        }
        // Hour cleanup uses the existing lifecycle owner, including active leases
        // and journal obligations. Never discard an ended-hour entry directly.
    }

    fn prune_prepared_hours(self: &Arc<Self>, current_hour: i64) {
        let candidates = {
            let memory = self.memory.lock().expect("invocation range memory");
            memory
                .entries
                .iter()
                .filter(|(owner, _)| owner.hour().is_some_and(|hour| hour < current_hour))
                .filter_map(|(owner, entry)| {
                    entry.prefix.clone().map(|prefix| (owner.clone(), prefix))
                })
                .collect::<Vec<_>>()
        };
        for (owner, prefix) in candidates {
            // This memory retirement does not lower the durable reservation
            // ceiling. Re-entry skips discarded unused IDs, preserving uniqueness.
            drop(self.freeze_owner(owner, &prefix));
        }
    }

    async fn prepare_owner(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        owner: Owner,
    ) -> PreparationOutcome {
        let generation = {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if let Some(entry) = memory.entries.get(&owner) {
                entry.generation
            } else {
                memory.generation += 1;
                memory.generation
            }
        };
        self.prepare_owner_generation(pool, owner, generation).await
    }

    async fn prepare_owner_generation(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        owner: Owner,
        generation: u64,
    ) -> PreparationOutcome {
        let generation = {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if let Some(entry) = memory.entries.get(&owner) {
                return if entry.prefix.is_some() && !entry.retiring && entry.failure.is_none() {
                    PreparationOutcome::Ready
                } else {
                    PreparationOutcome::Deferred
                };
            }
            if !memory.allocations.is_empty()
                || (matches!(owner, Owner::Conversation(_)) && memory.occupancy() >= memory.target)
            {
                record_recovery(RecoveryDiagnostic::RangePrewarmDeferred);
                return PreparationOutcome::Deferred;
            }
            memory.generation = memory.generation.max(generation);
            let mut entry = Entry::initializing(generation);
            entry.speculative = true;
            memory.entries.insert(owner.clone(), entry);
            generation
        };
        // Do not reserve writer admission while waiting for a pool connection.
        // The actual reservation still rechecks availability after admission.
        let idle = pool.try_acquire();
        let pool_ready = idle.is_some();
        if let Some(mut connection) = idle {
            connection.return_to_pool().await;
        }
        let permit = if pool_ready {
            proxy_sqlite_write_coordinator().try_acquire(ProxySqliteWriteClass::P2Derived)
        } else {
            None
        };
        if let Some(permit) = permit {
            let outcome = self
                .initialize(pool, owner.clone(), generation, Some(permit), true)
                .await;
            let mut memory = self.memory.lock().expect("invocation range memory");
            let ready = memory.entries.get(&owner).is_some_and(|entry| {
                entry.generation == generation && entry.prefix.is_some() && entry.failure.is_none()
            });
            record_recovery(if ready {
                RecoveryDiagnostic::RangePrewarmReady
            } else {
                RecoveryDiagnostic::RangePrewarmDeferred
            });
            if !ready
                && memory
                    .entries
                    .get(&owner)
                    .is_some_and(|entry| entry.speculative)
                && let Some(entry) = memory.entries.remove(&owner)
            {
                entry.notify.notify_waiters();
                self.admission.notify_waiters();
            }
            if ready {
                PreparationOutcome::Ready
            } else {
                outcome
            }
        } else {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if memory
                .entries
                .get(&owner)
                .is_some_and(|entry| entry.generation == generation)
                && let Some(entry) = memory.entries.remove(&owner)
            {
                entry.notify.notify_waiters();
                self.admission.notify_waiters();
            }
            record_recovery(RecoveryDiagnostic::RangePrewarmDeferred);
            PreparationOutcome::Deferred
        }
    }

    #[cfg(test)]
    pub(crate) async fn test_prepare_at(self: &Arc<Self>, pool: &Pool<Sqlite>, now: DateTime<Utc>) {
        self.pool.get_or_init(|| pool.clone());
        // Establish this fixture's idle-pool precondition; production preparation
        // skips rather than waiting when the pool is busy.
        pool.acquire().await.unwrap().return_to_pool().await;
        self.prepare_turn(
            pool,
            now,
            &mut Default::default(),
            &CancellationToken::new(),
        )
        .await;
    }

    #[cfg(test)]
    pub(crate) async fn test_prepare_key(self: &Arc<Self>, pool: &Pool<Sqlite>, key: &str) -> bool {
        self.pool.get_or_init(|| pool.clone());
        pool.acquire().await.unwrap().return_to_pool().await;
        matches!(
            self.prepare_owner(pool, Owner::Conversation(key.to_owned()))
                .await,
            PreparationOutcome::Ready
        )
    }

    #[cfg(test)]
    pub(crate) async fn test_prepare_keys(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        keys: &[&str],
    ) -> usize {
        let mut owners = keys
            .iter()
            .map(|key| Owner::Conversation((*key).to_owned()))
            .collect();
        self.prepare_turn(pool, Utc::now(), &mut owners, &CancellationToken::new())
            .await;
        with_retry_queue(self, |queue| queue.len())
    }

    #[cfg(test)]
    pub(crate) async fn test_retry_preparations(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        now: DateTime<Utc>,
    ) -> usize {
        self.pool.get_or_init(|| pool.clone());
        pool.acquire().await.unwrap().return_to_pool().await;
        with_retry_queue(self, |queue| queue.make_due());
        self.prepare_turn(
            pool,
            now,
            &mut Default::default(),
            &CancellationToken::new(),
        )
        .await;
        with_retry_queue(self, |queue| queue.len())
    }

    #[cfg(test)]
    pub(crate) fn test_pending_retry_count(self: &Arc<Self>) -> usize {
        with_retry_queue(self, |queue| queue.pending.len())
    }

    #[cfg(test)]
    pub(crate) fn test_deferred_retry_count(self: &Arc<Self>) -> usize {
        with_retry_queue(self, |queue| {
            queue
                .pending
                .iter()
                .filter(|item| item.attempts > 0)
                .count()
        })
    }

    #[cfg(test)]
    pub(crate) fn test_set_preparation_target(self: &Arc<Self>, target: usize) {
        self.memory.lock().expect("invocation range memory").target = target;
    }
}
