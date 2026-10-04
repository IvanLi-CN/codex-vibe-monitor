//! Durable reservation authority shared by conversation and hourly invocation owners.

use super::*;
use tokio::sync::Notify;

const RANGE_SIZE: u32 = 64;
const WAIT_BUDGET: Duration = Duration::from_millis(100);

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

#[derive(Debug, Default)]
struct Memory {
    entries: HashMap<Owner, Entry>,
    generation: u64,
    batch_running: bool,
}

/// Memory is the issuance authority; only committed SQLite ranges may enter it.
/// No memory guard survives an await, including on initialization and refill.
#[derive(Debug, Default)]
pub(crate) struct InvocationRangeManager {
    memory: std::sync::Mutex<Memory>,
}

#[derive(Debug, Clone)]
struct Reservation {
    owner: Owner,
    prefix: String,
    generation: u64,
    floor: i64,
}

impl InvocationRangeManager {
    pub(crate) async fn allocate(
        self: &Arc<Self>,
        pool: &Pool<Sqlite>,
        key: Option<&str>,
    ) -> Result<String> {
        self.allocate_owner(pool, Owner::from_key(key)).await
    }

    async fn allocate_owner(self: &Arc<Self>, pool: &Pool<Sqlite>, owner: Owner) -> Result<String> {
        let deadline = tokio::time::Instant::now() + WAIT_BUDGET;
        loop {
            let (notify, initialization, issued, trigger) = {
                let mut memory = self.memory.lock().expect("invocation range memory");
                let initialization = if !memory.entries.contains_key(&owner) {
                    memory.generation += 1;
                    let generation = memory.generation;
                    memory
                        .entries
                        .insert(owner.clone(), Entry::initializing(generation));
                    Some(generation)
                } else {
                    None
                };
                let entry = memory
                    .entries
                    .get_mut(&owner)
                    .expect("admitted invocation owner");
                if let Some(failure) = &entry.failure {
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
                (
                    entry.notify.clone(),
                    initialization,
                    issued,
                    entry.can_refill(32),
                )
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
