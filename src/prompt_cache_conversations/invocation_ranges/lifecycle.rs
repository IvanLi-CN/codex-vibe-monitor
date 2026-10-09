//! Short memory namespace fences, including terminal records not yet reconciled.
use super::*;

#[derive(Default)]
struct Namespace {
    occupied: HashMap<String, usize>,
    pending_prefixes: HashMap<String, usize>,
    pending_ids: HashMap<String, usize>,
}

static NAMESPACE: Lazy<std::sync::Mutex<Namespace>> =
    Lazy::new(|| std::sync::Mutex::new(Namespace::default()));

fn decrement(map: &mut HashMap<String, usize>, key: &str) {
    if let Some(count) = map.get_mut(key) {
        *count -= 1;
        if *count == 0 {
            map.remove(key);
        }
    }
}

#[derive(Debug)]
pub(crate) struct PrefixGuard(String);

impl PrefixGuard {
    pub(crate) fn for_invocation(id: &str) -> Option<Self> {
        (id.len() == PROXY_INVOKE_ID_LENGTH && id.is_ascii()).then(|| Self::new(&id[..6]))
    }

    pub(super) fn new(prefix: &str) -> Self {
        *NAMESPACE
            .lock()
            .expect("invocation namespace")
            .occupied
            .entry(prefix.to_owned())
            .or_default() += 1;
        Self(prefix.to_owned())
    }
}

impl Drop for PrefixGuard {
    fn drop(&mut self) {
        decrement(
            &mut NAMESPACE.lock().expect("invocation namespace").occupied,
            &self.0,
        );
    }
}

pub(crate) fn prefix_occupied(prefix: &str) -> bool {
    NAMESPACE
        .lock()
        .expect("invocation namespace")
        .occupied
        .contains_key(prefix)
}

fn prefix_owner_count(prefix: &str) -> usize {
    NAMESPACE
        .lock()
        .expect("invocation namespace")
        .occupied
        .get(prefix)
        .copied()
        .unwrap_or_default()
}

pub(crate) fn pending_prefix(prefix: &str) -> bool {
    NAMESPACE
        .lock()
        .expect("invocation namespace")
        .pending_prefixes
        .contains_key(prefix)
}

pub(crate) fn pending_floor(prefix: &str) -> i64 {
    NAMESPACE
        .lock()
        .expect("invocation namespace")
        .pending_ids
        .keys()
        .filter_map(|id| {
            id.strip_prefix(prefix)
                .and_then(decode_prompt_cache_conversation_sequence)
        })
        .map(i64::from)
        .max()
        .unwrap_or(-1)
}

#[derive(Debug)]
struct PendingIdentityGuard {
    id: String,
    prefix: PrefixGuard,
}

impl PendingIdentityGuard {
    fn new(id: &str) -> Option<Self> {
        if id.len() != PROXY_INVOKE_ID_LENGTH || !id.is_ascii() {
            return None;
        }
        let prefix = id[..6].to_owned();
        let mut namespace = NAMESPACE.lock().expect("invocation namespace");
        *namespace.occupied.entry(prefix.clone()).or_default() += 1;
        *namespace
            .pending_prefixes
            .entry(prefix.clone())
            .or_default() += 1;
        *namespace.pending_ids.entry(id.to_owned()).or_default() += 1;
        Some(Self {
            id: id.to_owned(),
            prefix: PrefixGuard(prefix),
        })
    }
}

impl Drop for PendingIdentityGuard {
    fn drop(&mut self) {
        let mut namespace = NAMESPACE.lock().expect("invocation namespace");
        decrement(&mut namespace.pending_ids, &self.id);
        decrement(&mut namespace.pending_prefixes, &self.prefix.0);
    }
}

pub(crate) type TerminalIdentity = (String, String, bool);

#[derive(Debug, Default)]
pub(crate) struct PendingIdentityRegistry {
    identities: std::sync::Mutex<HashMap<TerminalIdentity, PendingIdentityGuard>>,
}

impl PendingIdentityRegistry {
    pub(crate) fn register(&self, id: &str, occurred_at: &str, raw: bool) {
        let mut identities = self
            .identities
            .lock()
            .expect("pending invocation identities");
        let key = (id.to_owned(), occurred_at.to_owned(), raw);
        if let std::collections::hash_map::Entry::Vacant(entry) = identities.entry(key)
            && let Some(guard) = PendingIdentityGuard::new(id)
        {
            entry.insert(guard);
        }
    }

    pub(crate) fn acknowledge(&self, id: &str, occurred_at: &str, raw: bool) {
        self.identities
            .lock()
            .expect("pending invocation identities")
            .remove(&(id.to_owned(), occurred_at.to_owned(), raw));
    }
}

pub(crate) struct LifecycleFence {
    manager: Arc<InvocationRangeManager>,
    owner: Owner,
    generation: u64,
}

impl Drop for LifecycleFence {
    fn drop(&mut self) {
        let mut memory = self.manager.memory.lock().expect("invocation range memory");
        if memory
            .entries
            .get(&self.owner)
            .is_some_and(|entry| entry.generation == self.generation && entry.retiring)
            && let Some(entry) = memory.entries.remove(&self.owner)
        {
            debug!(owner_type = self.owner.kind(), utc_hour = self.owner.hour(), prefix = ?entry.prefix, generation = self.generation, reserved_ceiling = entry.ceiling, issued_floor = entry.issued_floor, "invocation lifecycle fence drained; allocation generation invalidated");
            entry.notify.notify_waiters();
        }
        self.manager.admission.notify_waiters();
    }
}

/// Own the SQLite worker alongside its fences. Cancellation drains/closes the
/// connection before any waiter may install a replacement owner generation.
pub(crate) struct LifecycleConnection {
    pub(crate) connection: Option<sqlx::pool::PoolConnection<Sqlite>>,
    pub(crate) fences: Vec<LifecycleFence>,
}

impl LifecycleConnection {
    pub(crate) fn new(fences: Vec<LifecycleFence>) -> Self {
        Self {
            connection: None,
            fences,
        }
    }

    pub(crate) fn connection(&mut self) -> &mut SqliteConnection {
        self.connection
            .as_mut()
            .expect("lifecycle SQLite connection")
            .as_mut()
    }

    pub(crate) fn committed(mut self) {
        drop(self.connection.take());
        self.fences.clear();
    }
}

impl Drop for LifecycleConnection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };
        let fences = std::mem::take(&mut self.fences);
        tokio::spawn(async move {
            if let Err(error) = connection.close().await {
                warn!(error = %error, "invocation lifecycle connection close failed");
            }
            drop(fences);
        });
    }
}

impl InvocationRangeManager {
    pub(crate) fn owner_release_eligible(&self, owner: &Owner, prefix: &str) -> bool {
        let memory = self.memory.lock().expect("invocation range memory");
        !(memory.allocations.contains_key(owner)
            || memory.active_prefixes.contains_key(prefix)
            || matches!(owner, Owner::Conversation(key) if memory.leases.contains_key(key))
            || pending_prefix(prefix)
            || prefix_owner_count(prefix)
                > usize::from(
                    memory
                        .entries
                        .get(owner)
                        .is_some_and(|entry| entry.namespace.is_some()),
                )
            || memory.entries.get(owner).is_some_and(|entry| {
                entry.operation
                    || entry.retiring
                    || entry
                        .prefix
                        .as_deref()
                        .is_some_and(|existing| existing != prefix)
            }))
    }

    pub(crate) fn freeze_owner(
        self: &Arc<Self>,
        owner: Owner,
        prefix: &str,
    ) -> Option<LifecycleFence> {
        let mut memory = self.memory.lock().expect("invocation range memory");
        let protected = memory.allocations.contains_key(&owner)
            || memory.active_prefixes.contains_key(prefix)
            || matches!(&owner, Owner::Conversation(key) if memory.leases.contains_key(key))
            || pending_prefix(prefix)
            || prefix_owner_count(prefix)
                > usize::from(
                    memory
                        .entries
                        .get(&owner)
                        .is_some_and(|entry| entry.namespace.is_some()),
                )
            || memory.entries.get(&owner).is_some_and(|entry| {
                entry.operation
                    || entry.retiring
                    || entry
                        .prefix
                        .as_deref()
                        .is_some_and(|existing| existing != prefix)
            });
        if protected {
            debug!(
                owner_type = owner.kind(),
                utc_hour = owner.hour(),
                prefix,
                "invocation lifecycle release deferred by active, pending or range operation reference"
            );
            return None;
        }
        if !memory.entries.contains_key(&owner) {
            memory.generation += 1;
            let mut entry = Entry::initializing(memory.generation);
            entry.prefix = Some(prefix.to_owned());
            entry.namespace = Some(PrefixGuard::new(prefix));
            memory.entries.insert(owner.clone(), entry);
        }
        let entry = memory.entries.get_mut(&owner).unwrap();
        entry.retiring = true;
        entry.operation = true;
        Some(LifecycleFence {
            manager: self.clone(),
            owner,
            generation: entry.generation,
        })
    }

    pub(crate) async fn cleanup_hours(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        dry_run: bool,
    ) -> Result<HourlyIdentityCleanupResult> {
        use crate::maintenance::identity_cleanup_query as bounded;
        const SCOPE: &str = "invocation_hour_prefixes";
        let current_hour = Utc::now().timestamp().div_euclid(3600);
        let cursor: Option<String> = if dry_run {
            None
        } else {
            bounded(sqlx::query_scalar("SELECT cursor_key FROM prompt_cache_conversation_orphan_cleanup_state WHERE scope=?")
                .bind(SCOPE).fetch_optional(pool)).await?.flatten()
        };
        let cursor = cursor.as_deref().map(str::parse::<i64>).transpose()?;
        let rows = bounded(sqlx::query_as::<_, (i64, String)>("SELECT utc_hour,prefix FROM hourly_invoke_prefixes WHERE utc_hour<?1 AND (?2 IS NULL OR utc_hour>?2) ORDER BY utc_hour LIMIT 32")
            .bind(current_hour).bind(cursor).fetch_all(pool)).await?;
        let mut result = HourlyIdentityCleanupResult {
            checked: rows.len(),
            released: 0,
            has_more: rows.len() == 32,
        };
        let next_cursor = result.has_more.then(|| rows.last().unwrap().0.to_string());
        let mut fences = Vec::new();
        let mut eligible = Vec::new();
        for (hour, prefix) in rows {
            if !self.owner_release_eligible(&Owner::Hour(hour), &prefix) {
                continue;
            }
            let fence = if dry_run {
                None
            } else {
                self.freeze_owner(Owner::Hour(hour), &prefix)
            };
            if !dry_run && fence.is_none() {
                continue;
            }
            let referenced: bool = bounded(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM codex_invocations WHERE length(invoke_id)=10 AND invoke_id>=?1 AND invoke_id < (?1 || '['))")
                .bind(&prefix).fetch_one(pool)).await?;
            if !referenced {
                eligible.push((hour, prefix));
                if let Some(fence) = fence {
                    fences.push(fence);
                }
            }
        }
        if dry_run {
            result.released = eligible.len();
            return Ok(result);
        }
        let mut scope = LifecycleConnection::new(fences);
        scope.connection = Some(bounded(pool.acquire()).await?);
        bounded(sqlx::query("BEGIN IMMEDIATE").execute(scope.connection())).await?;
        for (hour, prefix) in eligible {
            result.released += bounded(sqlx::query("DELETE FROM hourly_invoke_prefixes WHERE utc_hour=?1 AND prefix=?2 AND NOT EXISTS(SELECT 1 FROM codex_invocations WHERE length(invoke_id)=10 AND invoke_id>=?2 AND invoke_id < (?2 || '['))")
                .bind(hour).bind(prefix).execute(scope.connection())).await?.rows_affected() as usize;
        }
        bounded(sqlx::query("INSERT INTO prompt_cache_conversation_orphan_cleanup_state(scope,cursor_key,epoch,updated_at) VALUES(?,?,?,STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(scope) DO UPDATE SET cursor_key=excluded.cursor_key,epoch=epoch+excluded.epoch,updated_at=excluded.updated_at")
            .bind(SCOPE).bind(next_cursor).bind(i64::from(!result.has_more)).execute(scope.connection())).await?;
        bounded(sqlx::query("COMMIT").execute(scope.connection())).await?;
        scope.committed();
        Ok(result)
    }
}

#[derive(Debug, Default)]
pub(crate) struct HourlyIdentityCleanupResult {
    pub(crate) checked: usize,
    pub(crate) released: usize,
    pub(crate) has_more: bool,
}
