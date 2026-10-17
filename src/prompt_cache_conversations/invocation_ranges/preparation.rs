//! Best-effort preparation never queues ahead of actual missing-range demand.
use super::*;

pub(super) const STARTUP_OWNER_LIMIT: usize = 128;

fn hours(now: DateTime<Utc>) -> impl Iterator<Item = Owner> {
    let hour = now.timestamp().div_euclid(3600);
    let next = (now.timestamp().rem_euclid(3600) >= 3540).then_some(Owner::Hour(hour + 1));
    std::iter::once(Owner::Hour(hour)).chain(next)
}

impl InvocationRangeManager {
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
        for _ in 0..MAX_REFILL_BATCH_SIZE {
            if shutdown.is_cancelled() {
                break;
            }
            let Some(owner) = owners.front().cloned() else {
                break;
            };
            if !self.prepare_owner(pool, owner).await {
                break;
            }
            owners.pop_front();
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

    async fn prepare_owner(self: &Arc<Self>, pool: &Pool<Sqlite>, owner: Owner) -> bool {
        let generation = {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if let Some(entry) = memory.entries.get(&owner) {
                return !entry.retiring;
            }
            if !memory.allocations.is_empty()
                || (matches!(owner, Owner::Conversation(_)) && memory.occupancy() >= memory.target)
            {
                record_recovery(RecoveryDiagnostic::RangePrewarmDeferred);
                return false;
            }
            memory.generation += 1;
            let generation = memory.generation;
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
            self.initialize(pool, owner.clone(), generation, Some(permit), true)
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
            }
            ready
        } else {
            let mut memory = self.memory.lock().expect("invocation range memory");
            if memory
                .entries
                .get(&owner)
                .is_some_and(|entry| entry.generation == generation)
                && let Some(entry) = memory.entries.remove(&owner)
            {
                entry.notify.notify_waiters();
            }
            record_recovery(RecoveryDiagnostic::RangePrewarmDeferred);
            false
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
        self.prepare_owner(pool, Owner::Conversation(key.to_owned()))
            .await
    }
}
