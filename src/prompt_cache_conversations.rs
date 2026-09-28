use super::*;

pub(crate) const PROMPT_CACHE_CONVERSATION_ID_LENGTH: usize = 6;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH: usize = 4;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX: u32 = 31;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY: u32 =
    PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX.pow(PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH as u32);
const PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS: usize = 5;
const PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME: &str = "prompt_cache_conversations_v1";
const PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES: i64 = 5;
const PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY: usize = 4096;

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationIdentity {
    pub(crate) conversation_id: String,
    pub(crate) next_sequence: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptCacheConversationIdentityCache {
    pub(crate) conversations: HashMap<String, PromptCacheConversationIdentity>,
    pub(crate) active_prompt_cache_keys: HashMap<String, usize>,
    pub(crate) unbound_prefix: Option<UnboundInvokePrefix>,
}

#[derive(Debug, Clone)]
pub(crate) struct UnboundInvokePrefix {
    pub(crate) hour_key: i64,
    pub(crate) prefix: String,
    pub(crate) next_sequence: u32,
}

#[derive(Debug, FromRow)]
struct PromptCacheConversationStatsRow {
    prompt_cache_key: String,
    conversation_id: String,
    max_invoke_id: Option<String>,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
    reported_cache_write_tokens: i64,
    reasoning_tokens: i64,
    total_tokens: i64,
    cost: f64,
    cost_input: f64,
    cost_cache_write: f64,
    cost_cache_read: f64,
    cost_output: f64,
    cost_reasoning: f64,
    first_invocation_at: Option<String>,
    last_invocation_at: Option<String>,
}

pub(crate) fn prompt_cache_conversation_id_from_invoke_id(invoke_id: &str) -> Option<&str> {
    (invoke_id.len() == PROXY_INVOKE_ID_LENGTH
        && invoke_id
            .chars()
            .all(|character| PROXY_INVOKE_ID_ALPHABET.contains(&character)))
    .then(|| invoke_id.get(..PROMPT_CACHE_CONVERSATION_ID_LENGTH))
    .flatten()
}

pub(crate) fn encode_prompt_cache_conversation_sequence(sequence: u32) -> Result<String> {
    if sequence >= PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY {
        bail!(
            "prompt-cache conversation invoke sequence overflow: sequence={sequence} capacity={}",
            PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY
        );
    }

    let mut value = sequence;
    let mut encoded = [PROXY_INVOKE_ID_ALPHABET[0]; PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH];
    for slot in encoded.iter_mut().rev() {
        *slot =
            PROXY_INVOKE_ID_ALPHABET[(value % PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX) as usize];
        value /= PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX;
    }
    Ok(encoded.iter().collect())
}

fn decode_prompt_cache_conversation_sequence(value: &str) -> Option<u32> {
    if value.len() != PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH {
        return None;
    }
    value.chars().try_fold(0_u32, |accumulator, character| {
        let digit = PROXY_INVOKE_ID_ALPHABET
            .iter()
            .position(|candidate| *candidate == character)? as u32;
        accumulator
            .checked_mul(PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX)?
            .checked_add(digit)
    })
}

fn generate_prompt_cache_conversation_id() -> String {
    nanoid::nanoid!(
        PROMPT_CACHE_CONVERSATION_ID_LENGTH,
        &PROXY_INVOKE_ID_ALPHABET
    )
}

fn normalize_prompt_cache_key(prompt_cache_key: Option<&str>) -> Option<&str> {
    prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn cache_prompt_cache_conversation_identity(
    cache: &mut PromptCacheConversationIdentityCache,
    prompt_cache_key: &str,
    identity: PromptCacheConversationIdentity,
) -> bool {
    if cache.conversations.contains_key(prompt_cache_key) {
        return true;
    }
    if cache.conversations.len() >= PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY {
        let evicted_key = cache
            .conversations
            .keys()
            .find(|key| !cache.active_prompt_cache_keys.contains_key(key.as_str()))
            .cloned();
        if let Some(evicted_key) = evicted_key {
            cache.conversations.remove(&evicted_key);
        }
    }
    if cache.conversations.len() >= PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY {
        return false;
    }
    cache
        .conversations
        .insert(prompt_cache_key.to_string(), identity);
    true
}

fn trim_prompt_cache_conversation_identity_cache(cache: &mut PromptCacheConversationIdentityCache) {
    while cache.conversations.len() > PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY {
        let Some(evicted_key) = cache
            .conversations
            .keys()
            .find(|key| !cache.active_prompt_cache_keys.contains_key(key.as_str()))
            .cloned()
        else {
            break;
        };
        cache.conversations.remove(&evicted_key);
    }
}

pub(crate) fn prompt_cache_key_fingerprint(prompt_cache_key: &str) -> String {
    let digest = Sha256::digest(prompt_cache_key.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) async fn ensure_prompt_cache_conversations_schema(pool: &Pool<Sqlite>) -> Result<usize> {
    let conversation_id_alphabet = PROXY_INVOKE_ID_ALPHABET.iter().collect::<String>();
    let schema_sql = format!(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_conversations (
            conversation_id TEXT PRIMARY KEY,
            prompt_cache_key TEXT NOT NULL UNIQUE,
            last_invoke_sequence INTEGER NOT NULL DEFAULT -1,
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_input_tokens INTEGER NOT NULL DEFAULT 0,
            reported_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            cost REAL NOT NULL DEFAULT 0,
            cost_input REAL NOT NULL DEFAULT 0,
            cost_cache_write REAL NOT NULL DEFAULT 0,
            cost_cache_read REAL NOT NULL DEFAULT 0,
            cost_output REAL NOT NULL DEFAULT 0,
            cost_reasoning REAL NOT NULL DEFAULT 0,
            first_invocation_at TEXT,
            last_invocation_at TEXT,
            created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
            updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
            CHECK (length(conversation_id) = 6),
            CHECK (conversation_id NOT GLOB '*[^{conversation_id_alphabet}]*'),
            CHECK (length(trim(prompt_cache_key)) > 0)
        )
        "#
    );
    sqlx::query(&schema_sql)
        .execute(pool)
        .await
        .context("failed to ensure prompt_cache_conversations table existence")?;

    for (name, expression) in [
        (
            "idx_prompt_cache_conversations_last_invocation",
            "prompt_cache_conversations (last_invocation_at DESC, conversation_id)",
        ),
        (
            "idx_prompt_cache_conversations_request_count",
            "prompt_cache_conversations (request_count DESC, conversation_id)",
        ),
    ] {
        sqlx::query(&format!(
            "CREATE INDEX IF NOT EXISTS {name} ON {expression}"
        ))
        .execute(pool)
        .await
        .with_context(|| format!("failed to ensure index {name}"))?;
    }

    let migration_already_completed = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM schema_refresh_migrations WHERE migration_name = ?1
        )
        "#,
    )
    .bind(PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME)
    .fetch_one(pool)
    .await?
        != 0;
    if migration_already_completed {
        let conversation_table_populated =
            sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM prompt_cache_conversations)")
                .fetch_one(pool)
                .await?
                != 0;
        if conversation_table_populated {
            return Ok(0);
        }
        // Keep the recovery path for a manually cleared conversation table. The normal startup
        // path above is a single-row existence check instead of a full invocation reconciliation.
        warn!(
            migration = PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME,
            "prompt-cache conversation table is empty after a completed backfill; rebuilding identities"
        );
    }

    let prompt_cache_keys = sqlx::query_scalar::<_, String>(&format!(
        "SELECT DISTINCT {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} AS prompt_cache_key \
         FROM codex_invocations \
         WHERE {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
           AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} <> '' \
         ORDER BY prompt_cache_key"
    ))
    .fetch_all(pool)
    .await
    .context("failed to enumerate prompt-cache conversation backfill keys")?;
    info!(
        migration = PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME,
        candidate_keys = prompt_cache_keys.len(),
        "prompt-cache conversation identity backfill started"
    );

    let mut backfilled = 0;
    let mut backfill_stats_keys = HashSet::new();
    for prompt_cache_key in prompt_cache_keys {
        backfill_stats_keys.insert(prompt_cache_key.clone());
        if load_prompt_cache_conversation_row(pool, &prompt_cache_key)
            .await?
            .is_some()
        {
            continue;
        }
        create_prompt_cache_conversation_row(pool, &prompt_cache_key).await?;
        backfilled += 1;
    }

    if !backfill_stats_keys.is_empty() {
        refresh_prompt_cache_conversation_stats(pool, &backfill_stats_keys)
            .await
            .context("failed to materialize prompt-cache conversation backfill statistics")?;
    }

    sqlx::query("INSERT OR REPLACE INTO schema_refresh_migrations (migration_name) VALUES (?1)")
        .bind(PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME)
        .execute(pool)
        .await
        .context("failed to record prompt-cache conversation backfill")?;
    info!(
        backfilled,
        "prompt-cache conversation identity backfill completed"
    );
    Ok(backfilled)
}

async fn load_prompt_cache_conversation_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<Option<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT conversation_id, last_invoke_sequence \
         FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_optional(pool)
    .await
    .context("failed to load prompt-cache conversation identity")
}

async fn conversation_id_exists(pool: &Pool<Sqlite>, conversation_id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM prompt_cache_conversations WHERE conversation_id = ?1)",
    )
    .bind(conversation_id)
    .fetch_one(pool)
    .await?
        != 0)
}

async fn conversation_prefix_conflicts_with_live_invocation(
    pool: &Pool<Sqlite>,
    conversation_id: &str,
) -> Result<bool> {
    // The generated suffix alphabet is A-Z followed by digits. '[' is the
    // first byte after that alphabet in SQLite's BINARY collation.
    let upper_bound = format!("{conversation_id}[");
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(\
            SELECT 1 FROM codex_invocations \
            WHERE invoke_id >= ?1 AND invoke_id < ?2 AND length(invoke_id) = ?3
        )",
    )
    .bind(conversation_id)
    .bind(upper_bound)
    .bind(PROXY_INVOKE_ID_LENGTH as i64)
    .fetch_one(pool)
    .await?
        != 0)
}

async fn create_prompt_cache_conversation_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<PromptCacheConversationIdentity> {
    create_prompt_cache_conversation_row_with_generator(
        pool,
        prompt_cache_key,
        generate_prompt_cache_conversation_id,
    )
    .await
}

async fn create_prompt_cache_conversation_row_with_generator<F>(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    mut generate: F,
) -> Result<PromptCacheConversationIdentity>
where
    F: FnMut() -> String,
{
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let conversation_id = generate();
        if conversation_id_exists(pool, &conversation_id).await?
            || conversation_prefix_conflicts_with_live_invocation(pool, &conversation_id).await?
        {
            debug!(
                attempt,
                conversation_id = %conversation_id,
                prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                "prompt-cache conversation id candidate rejected"
            );
            continue;
        }

        let insert_result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO prompt_cache_conversations (
                conversation_id,
                prompt_cache_key
            ) VALUES (?1, ?2)
            "#,
        )
        .bind(&conversation_id)
        .bind(prompt_cache_key)
        .execute(pool)
        .await
        .with_context(|| {
            format!(
                "failed to persist prompt-cache conversation identity for key fingerprint {}",
                prompt_cache_key_fingerprint(prompt_cache_key)
            )
        })?;

        if insert_result.rows_affected() == 0 {
            if let Some((existing_conversation_id, last_invoke_sequence)) =
                load_prompt_cache_conversation_row(pool, prompt_cache_key).await?
            {
                debug!(
                    conversation_id = %existing_conversation_id,
                    prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                    "prompt-cache conversation identity creation recovered a concurrent row"
                );
                return recover_prompt_cache_conversation_identity(
                    pool,
                    existing_conversation_id,
                    last_invoke_sequence,
                )
                .await;
            }
            debug!(
                attempt,
                conversation_id = %conversation_id,
                prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                "prompt-cache conversation id candidate lost a database race; retrying"
            );
            continue;
        }

        info!(
            conversation_id = %conversation_id,
            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
            "prompt-cache conversation identity created"
        );
        return Ok(PromptCacheConversationIdentity {
            conversation_id,
            next_sequence: 0,
        });
    }

    bail!(
        "failed to allocate prompt-cache conversation id after {PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS} attempts"
    )
}

#[cfg(test)]
pub(crate) async fn create_prompt_cache_conversation_row_with_test_candidates(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    candidates: &[&str],
) -> Result<PromptCacheConversationIdentity> {
    let mut candidates = candidates
        .iter()
        .map(|candidate| (*candidate).to_string())
        .collect::<std::collections::VecDeque<_>>();
    create_prompt_cache_conversation_row_with_generator(pool, prompt_cache_key, move || {
        candidates
            .pop_front()
            .expect("test prompt-cache conversation candidate queue should not be exhausted")
    })
    .await
}

async fn recover_prompt_cache_conversation_identity(
    pool: &Pool<Sqlite>,
    conversation_id: String,
    last_invoke_sequence: i64,
) -> Result<PromptCacheConversationIdentity> {
    let live_next_sequence = max_live_sequence_for_conversation(pool, &conversation_id)
        .await?
        .and_then(|sequence| sequence.checked_add(1))
        .unwrap_or_default();
    let persisted_next_sequence =
        u32::try_from(last_invoke_sequence.saturating_add(1)).unwrap_or_default();
    Ok(PromptCacheConversationIdentity {
        conversation_id,
        next_sequence: live_next_sequence.max(persisted_next_sequence),
    })
}

async fn reserve_prompt_cache_conversation_sequence(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    next_sequence: u32,
) -> Result<Option<(String, u32)>> {
    let reservation_floor = i64::from(next_sequence) - 1;
    let maximum_sequence = i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY - 1);
    let mut transaction = pool
        .begin()
        .await
        .context("failed to begin prompt-cache conversation sequence reservation")?;
    let update_result = sqlx::query(
        r#"
        UPDATE prompt_cache_conversations
        SET last_invoke_sequence = MAX(last_invoke_sequence, ?1) + 1,
            updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE prompt_cache_key = ?2
          AND MAX(last_invoke_sequence, ?1) < ?3
        "#,
    )
    .bind(reservation_floor)
    .bind(prompt_cache_key)
    .bind(maximum_sequence)
    .execute(&mut *transaction)
    .await
    .context("failed to reserve prompt-cache conversation invoke sequence")?;
    let row = sqlx::query_as::<_, (String, i64)>(
        "SELECT conversation_id, last_invoke_sequence \
         FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_optional(&mut *transaction)
    .await
    .context("failed to read reserved prompt-cache conversation invoke sequence")?;
    transaction
        .commit()
        .await
        .context("failed to commit prompt-cache conversation sequence reservation")?;

    let Some((reserved_conversation_id, reserved_sequence)) = row else {
        return Ok(None);
    };
    if update_result.rows_affected() == 0 {
        if reserved_sequence >= maximum_sequence {
            bail!(
                "prompt-cache conversation invoke sequence overflow: sequence={} capacity={}",
                PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY,
                PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY
            );
        }
        bail!(
            "failed to reserve prompt-cache conversation invoke sequence for key fingerprint {}",
            prompt_cache_key_fingerprint(prompt_cache_key)
        );
    }
    let reserved_sequence = u32::try_from(reserved_sequence)
        .context("reserved prompt-cache conversation invoke sequence was out of range")?;
    if reserved_sequence >= PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY {
        bail!(
            "prompt-cache conversation invoke sequence overflow: sequence={} capacity={}",
            reserved_sequence,
            PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY
        );
    }
    Ok(Some((reserved_conversation_id, reserved_sequence)))
}

async fn max_live_sequence_for_conversation(
    pool: &Pool<Sqlite>,
    conversation_id: &str,
) -> Result<Option<u32>> {
    let invoke_ids = sqlx::query_scalar::<_, String>(
        "SELECT invoke_id FROM codex_invocations WHERE invoke_id LIKE ?1 || '%'",
    )
    .bind(conversation_id)
    .fetch_all(pool)
    .await
    .context("failed to recover prompt-cache conversation invoke sequence")?;
    Ok(invoke_ids
        .iter()
        .filter_map(|invoke_id| {
            let suffix = invoke_id.strip_prefix(conversation_id)?;
            (suffix.len() == PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH)
                .then(|| decode_prompt_cache_conversation_sequence(suffix))
                .flatten()
        })
        .max())
}

async fn load_or_create_prompt_cache_conversation_identity(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<PromptCacheConversationIdentity> {
    if let Some((conversation_id, last_invoke_sequence)) =
        load_prompt_cache_conversation_row(pool, prompt_cache_key).await?
    {
        return recover_prompt_cache_conversation_identity(
            pool,
            conversation_id,
            last_invoke_sequence,
        )
        .await;
    }
    create_prompt_cache_conversation_row(pool, prompt_cache_key).await
}

pub(crate) async fn allocate_proxy_invoke_id(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Result<String> {
    let mut cache = state.prompt_cache_conversation_cache.lock().await;
    allocate_proxy_invoke_id_locked(state, &mut cache, prompt_cache_key).await
}

pub(crate) async fn allocate_proxy_invoke_id_with_active_lease(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Result<String> {
    let mut cache = state.prompt_cache_conversation_cache.lock().await;
    let invoke_id = allocate_proxy_invoke_id_locked(state, &mut cache, prompt_cache_key).await?;
    if let Some(prompt_cache_key) = normalize_prompt_cache_key(prompt_cache_key) {
        let leases = cache
            .identity_cache
            .active_prompt_cache_keys
            .entry(prompt_cache_key.to_string())
            .or_default();
        *leases = leases.saturating_add(1);
    }
    Ok(invoke_id)
}

async fn allocate_proxy_invoke_id_locked(
    state: &AppState,
    cache: &mut PromptCacheConversationsCacheState,
    prompt_cache_key: Option<&str>,
) -> Result<String> {
    if let Some(prompt_cache_key) = normalize_prompt_cache_key(prompt_cache_key) {
        let (conversation_id, sequence) = 'reserve: {
            for recovery_attempt in 0..=1 {
                let (conversation_id, next_sequence) = {
                    let identity = if let Some(identity) =
                        cache.identity_cache.conversations.get(prompt_cache_key)
                    {
                        debug!(
                            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                            conversation_id = %identity.conversation_id,
                            "prompt-cache conversation identity cache hit"
                        );
                        identity.clone()
                    } else {
                        debug!(
                            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                            "prompt-cache conversation identity cache miss; loading from database"
                        );
                        let identity = match load_or_create_prompt_cache_conversation_identity(
                            &state.pool,
                            prompt_cache_key,
                        )
                        .await
                        {
                            Ok(identity) => identity,
                            Err(err) => {
                                error!(
                                    prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                                    error = %err,
                                    "failed to recover or create prompt-cache conversation identity"
                                );
                                return Err(err);
                            }
                        };
                        debug!(
                            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                            conversation_id = %identity.conversation_id,
                            "prompt-cache conversation identity recovered from database"
                        );
                        if !cache_prompt_cache_conversation_identity(
                            &mut cache.identity_cache,
                            prompt_cache_key,
                            identity.clone(),
                        ) {
                            debug!(
                                prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                                capacity = PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY,
                                "prompt-cache conversation identity cache is full; using durable identity only"
                            );
                        }
                        identity
                    };
                    (identity.conversation_id, identity.next_sequence)
                };
                if next_sequence >= PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY {
                    let err = encode_prompt_cache_conversation_sequence(next_sequence)
                        .expect_err("sequence capacity check should reject exhausted identity");
                    error!(
                        conversation_id = %conversation_id,
                        sequence = next_sequence,
                        error = %err,
                        "prompt-cache conversation invoke sequence exhausted"
                    );
                    return Err(err);
                }
                if let Some(reservation) = reserve_prompt_cache_conversation_sequence(
                    &state.pool,
                    prompt_cache_key,
                    next_sequence,
                )
                .await?
                {
                    break 'reserve reservation;
                }
                if recovery_attempt == 0 {
                    cache.identity_cache.conversations.remove(prompt_cache_key);
                    continue;
                }
                bail!(
                    "prompt-cache conversation identity disappeared during sequence reservation: key fingerprint {}",
                    prompt_cache_key_fingerprint(prompt_cache_key)
                );
            }
            unreachable!("prompt-cache conversation reservation loop should return or retry")
        };
        let suffix = encode_prompt_cache_conversation_sequence(sequence)?;
        if let Some(identity) = cache.identity_cache.conversations.get_mut(prompt_cache_key) {
            identity.conversation_id = conversation_id.clone();
            identity.next_sequence = sequence.saturating_add(1);
        }
        let invoke_id = format!("{}{}", conversation_id, suffix);
        debug!(
            conversation_id = %conversation_id,
            invoke_id = %invoke_id,
            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
            sequence,
            "allocated conversation-bound proxy invoke id"
        );
        return Ok(invoke_id);
    }

    let hour_key = Utc::now().timestamp().div_euclid(60 * 60);
    let prefix = if cache
        .identity_cache
        .unbound_prefix
        .as_ref()
        .is_some_and(|prefix| prefix.hour_key == hour_key)
    {
        cache
            .identity_cache
            .unbound_prefix
            .as_mut()
            .expect("current unbound invoke prefix should exist")
    } else {
        let prefix = loop {
            let candidate = generate_prompt_cache_conversation_id();
            if !conversation_prefix_conflicts_with_live_invocation(&state.pool, &candidate).await? {
                break candidate;
            }
            debug!(
                prefix = %candidate,
                prefix_hour = hour_key,
                "unbound proxy invoke prefix conflicts with a retained invocation"
            );
        };
        cache.identity_cache.unbound_prefix = Some(UnboundInvokePrefix {
            hour_key,
            prefix,
            next_sequence: 0,
        });
        cache
            .identity_cache
            .unbound_prefix
            .as_mut()
            .expect("unbound invoke prefix was initialized")
    };
    let sequence = prefix.next_sequence;
    let suffix = match encode_prompt_cache_conversation_sequence(sequence) {
        Ok(suffix) => suffix,
        Err(err) => {
            error!(
                prefix = %prefix.prefix,
                prefix_hour = hour_key,
                sequence,
                error = %err,
                "unbound proxy invoke sequence exhausted"
            );
            return Err(err);
        }
    };
    prefix.next_sequence = sequence.saturating_add(1);
    let invoke_id = format!("{}{}", prefix.prefix, suffix);
    debug!(
        invoke_id = %invoke_id,
        prefix_hour = hour_key,
        sequence,
        "allocated unbound proxy invoke id"
    );
    Ok(invoke_id)
}

const PROMPT_CACHE_CONVERSATION_STATS_REFRESH_ATTEMPTS: usize = 3;
const PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY: usize = 400;

pub(crate) async fn refresh_prompt_cache_conversation_stats(
    pool: &Pool<Sqlite>,
    prompt_cache_keys: &HashSet<String>,
) -> Result<usize> {
    let mut last_error = None;
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_STATS_REFRESH_ATTEMPTS {
        match refresh_prompt_cache_conversation_stats_once(pool, prompt_cache_keys).await {
            Ok(refreshed) => return Ok(refreshed),
            Err(error) => {
                warn!(
                    attempt,
                    max_attempts = PROMPT_CACHE_CONVERSATION_STATS_REFRESH_ATTEMPTS,
                    error = %error,
                    "prompt-cache conversation statistics refresh attempt failed"
                );
                last_error = Some(error);
                tokio::task::yield_now().await;
            }
        }
    }
    Err(last_error.expect("prompt-cache statistics refresh should record its last error"))
}

async fn refresh_prompt_cache_conversation_stats_once(
    pool: &Pool<Sqlite>,
    prompt_cache_keys: &HashSet<String>,
) -> Result<usize> {
    if prompt_cache_keys.is_empty() {
        return Ok(0);
    }
    let prompt_cache_keys = prompt_cache_keys.iter().collect::<Vec<_>>();
    let mut refreshed = 0;
    for prompt_cache_keys in
        prompt_cache_keys.chunks(PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY)
    {
        let mut tx = pool.begin().await?;
        let placeholders = std::iter::repeat_n("?", prompt_cache_keys.len())
            .collect::<Vec<_>>()
            .join(",");
        let success_like_sql = invocation_status_is_success_like_sql("i.status", "i.error_message");
        let stats_sql = format!(
            r#"
            SELECT
                c.prompt_cache_key AS prompt_cache_key,
                c.conversation_id AS conversation_id,
                MAX(CASE
                    WHEN length(i.invoke_id) = {PROXY_INVOKE_ID_LENGTH}
                     AND i.invoke_id >= c.conversation_id
                     AND i.invoke_id < (c.conversation_id || '[')
                    THEN i.invoke_id
                END) AS max_invoke_id,
                COUNT(i.id) AS request_count,
                COALESCE(SUM(CASE WHEN i.id IS NOT NULL AND ({success_like_sql}) THEN 1 ELSE 0 END), 0) AS success_count,
                COALESCE(SUM(CASE WHEN i.id IS NULL THEN 0 WHEN {success_like_sql} THEN 0 ELSE 1 END), 0) AS failure_count,
                COALESCE(SUM(i.input_tokens), 0) AS input_tokens,
                COALESCE(SUM(i.output_tokens), 0) AS output_tokens,
                COALESCE(SUM(i.cache_input_tokens), 0) AS cache_input_tokens,
                COALESCE(SUM(i.reported_cache_write_tokens), 0) AS reported_cache_write_tokens,
                COALESCE(SUM(i.reasoning_tokens), 0) AS reasoning_tokens,
                COALESCE(SUM(i.total_tokens), 0) AS total_tokens,
                COALESCE(SUM(CAST(i.cost AS REAL)), 0.0) AS cost,
                COALESCE(SUM(CAST(i.cost_input AS REAL)), 0.0) AS cost_input,
                COALESCE(SUM(CAST(i.cost_cache_write AS REAL)), 0.0) AS cost_cache_write,
                COALESCE(SUM(CAST(i.cost_cache_read AS REAL)), 0.0) AS cost_cache_read,
                COALESCE(SUM(CAST(i.cost_output AS REAL)), 0.0) AS cost_output,
                COALESCE(SUM(CAST(i.cost_reasoning AS REAL)), 0.0) AS cost_reasoning,
                MIN(i.occurred_at) AS first_invocation_at,
                MAX(i.occurred_at) AS last_invocation_at
            FROM prompt_cache_conversations AS c
            LEFT JOIN codex_invocations AS i
                ON {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} = c.prompt_cache_key
            WHERE c.prompt_cache_key IN ({placeholders})
            GROUP BY c.prompt_cache_key, c.conversation_id
            "#,
        );
        let mut stats_query = sqlx::query_as::<_, PromptCacheConversationStatsRow>(&stats_sql);
        for prompt_cache_key in prompt_cache_keys {
            stats_query = stats_query.bind(prompt_cache_key.as_str());
        }
        let stats_rows = stats_query.fetch_all(&mut *tx).await?;
        for stats in &stats_rows {
            let max_sequence = stats.max_invoke_id.as_deref().and_then(|invoke_id| {
                let suffix = invoke_id_suffix(invoke_id, &stats.conversation_id)?;
                decode_prompt_cache_conversation_sequence(suffix)
            });
            sqlx::query(
                r#"
                UPDATE prompt_cache_conversations
                SET last_invoke_sequence = MAX(last_invoke_sequence, COALESCE(?1, -1)),
                    request_count = ?2,
                    success_count = ?3,
                    failure_count = ?4,
                    input_tokens = ?5,
                    output_tokens = ?6,
                    cache_input_tokens = ?7,
                    reported_cache_write_tokens = ?8,
                    reasoning_tokens = ?9,
                    total_tokens = ?10,
                    cost = ?11,
                    cost_input = ?12,
                    cost_cache_write = ?13,
                    cost_cache_read = ?14,
                    cost_output = ?15,
                    cost_reasoning = ?16,
                    first_invocation_at = ?17,
                    last_invocation_at = ?18,
                    updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE prompt_cache_key = ?19
                "#,
            )
            .bind(max_sequence.map(i64::from))
            .bind(stats.request_count)
            .bind(stats.success_count)
            .bind(stats.failure_count)
            .bind(stats.input_tokens)
            .bind(stats.output_tokens)
            .bind(stats.cache_input_tokens)
            .bind(stats.reported_cache_write_tokens)
            .bind(stats.reasoning_tokens)
            .bind(stats.total_tokens)
            .bind(stats.cost)
            .bind(stats.cost_input)
            .bind(stats.cost_cache_write)
            .bind(stats.cost_cache_read)
            .bind(stats.cost_output)
            .bind(stats.cost_reasoning)
            .bind(stats.first_invocation_at.as_deref())
            .bind(stats.last_invocation_at.as_deref())
            .bind(&stats.prompt_cache_key)
            .execute(&mut *tx)
            .await?;
        }
        refreshed += stats_rows.len();
        tx.commit().await?;
    }
    debug!(
        keys = prompt_cache_keys.len(),
        refreshed, "prompt-cache conversation statistics refreshed"
    );
    Ok(refreshed)
}

pub(crate) async fn refresh_all_prompt_cache_conversation_stats(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let prompt_cache_keys = sqlx::query_scalar::<_, String>(
        "SELECT prompt_cache_key FROM prompt_cache_conversations ORDER BY prompt_cache_key",
    )
    .fetch_all(pool)
    .await
    .context("failed to enumerate prompt-cache conversation statistics refresh keys")?
    .into_iter()
    .collect::<HashSet<_>>();
    refresh_prompt_cache_conversation_stats(pool, &prompt_cache_keys).await
}

fn invoke_id_suffix<'a>(invoke_id: &'a str, conversation_id: &str) -> Option<&'a str> {
    let suffix = invoke_id.strip_prefix(conversation_id)?;
    (suffix.len() == PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH).then_some(suffix)
}

pub(crate) async fn cleanup_orphan_prompt_cache_conversations(
    pool: &Pool<Sqlite>,
    dry_run: bool,
) -> Result<usize> {
    cleanup_orphan_prompt_cache_conversations_with_active_keys(pool, dry_run, &HashSet::new()).await
}

async fn cleanup_orphan_prompt_cache_conversations_with_active_keys(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    active_prompt_cache_keys: &HashSet<String>,
) -> Result<usize> {
    let predicate = format!(
        "({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} = prompt_cache_conversations.prompt_cache_key OR (length(codex_invocations.invoke_id) = {PROXY_INVOKE_ID_LENGTH} AND codex_invocations.invoke_id >= prompt_cache_conversations.conversation_id AND codex_invocations.invoke_id < (prompt_cache_conversations.conversation_id || '[')))"
    );
    let orphan_predicate = format!(
        "(last_invocation_at IS NOT NULL OR created_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now', '-{PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES} minutes')) AND updated_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now', '-{PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES} minutes') AND NOT EXISTS (SELECT 1 FROM codex_invocations WHERE {predicate})"
    );
    if active_prompt_cache_keys.is_empty() {
        if dry_run {
            let count_sql =
                format!("SELECT COUNT(*) FROM prompt_cache_conversations WHERE {orphan_predicate}");
            return Ok(sqlx::query_scalar::<_, i64>(&count_sql)
                .fetch_one(pool)
                .await? as usize);
        }
        let delete_sql = format!("DELETE FROM prompt_cache_conversations WHERE {orphan_predicate}");
        return Ok(sqlx::query(&delete_sql)
            .execute(pool)
            .await?
            .rows_affected() as usize);
    }

    // Avoid expanding the active lease set into one SQLite statement. The cache mutex is held
    // by the caller for the mutating path, so this in-memory exclusion remains stable while the
    // bounded deletes re-check the orphan predicate.
    let candidate_sql =
        format!("SELECT prompt_cache_key FROM prompt_cache_conversations WHERE {orphan_predicate}");
    let candidate_keys = sqlx::query_scalar::<_, String>(&candidate_sql)
        .fetch_all(pool)
        .await?;
    let eligible_keys = candidate_keys
        .into_iter()
        .filter(|prompt_cache_key| !active_prompt_cache_keys.contains(prompt_cache_key))
        .collect::<Vec<_>>();
    if dry_run {
        return Ok(eligible_keys.len());
    }

    let mut released = 0_usize;
    for prompt_cache_keys in
        eligible_keys.chunks(PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY)
    {
        let placeholders = std::iter::repeat_n("?", prompt_cache_keys.len())
            .collect::<Vec<_>>()
            .join(",");
        let delete_sql = format!(
            "DELETE FROM prompt_cache_conversations WHERE {orphan_predicate} AND prompt_cache_key IN ({placeholders})"
        );
        let mut delete_query = sqlx::query(&delete_sql);
        for prompt_cache_key in prompt_cache_keys {
            delete_query = delete_query.bind(prompt_cache_key);
        }
        released =
            released.saturating_add(delete_query.execute(pool).await?.rows_affected() as usize);
    }
    Ok(released)
}

pub(crate) async fn cleanup_orphan_prompt_cache_conversations_with_cache(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) -> Result<usize> {
    let (active_prompt_cache_keys, released) = if dry_run {
        let active_prompt_cache_keys = cache
            .lock()
            .await
            .identity_cache
            .active_prompt_cache_keys
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let released = cleanup_orphan_prompt_cache_conversations_with_active_keys(
            pool,
            true,
            &active_prompt_cache_keys,
        )
        .await?;
        (active_prompt_cache_keys, released)
    } else {
        // Lease admission uses this mutex. Keep it through the single DELETE so a new
        // in-memory lease cannot arrive after the active-key exclusion is evaluated.
        let cache_state = cache.lock().await;
        let active_prompt_cache_keys = cache_state
            .identity_cache
            .active_prompt_cache_keys
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let released = cleanup_orphan_prompt_cache_conversations_with_active_keys(
            pool,
            false,
            &active_prompt_cache_keys,
        )
        .await?;
        drop(cache_state);
        (active_prompt_cache_keys, released)
    };
    if !dry_run && released > 0 {
        let existing_prompt_cache_keys = sqlx::query_scalar::<_, String>(
            "SELECT prompt_cache_key FROM prompt_cache_conversations",
        )
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
        let mut cache_state = cache.lock().await;
        cache_state
            .identity_cache
            .conversations
            .retain(|prompt_cache_key, _| existing_prompt_cache_keys.contains(prompt_cache_key));
        trim_prompt_cache_conversation_identity_cache(&mut cache_state.identity_cache);
        info!(
            released,
            active = active_prompt_cache_keys.len(),
            cached = cache_state.identity_cache.conversations.len(),
            "cleared released prompt-cache conversation identities"
        );
    }
    Ok(released)
}

pub(crate) async fn retain_active_prompt_cache_conversation(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
    prompt_cache_key: &str,
) {
    let mut cache_state = cache.lock().await;
    let prompt_cache_key = prompt_cache_key.trim();
    if prompt_cache_key.is_empty() {
        return;
    }
    cache_state
        .identity_cache
        .active_prompt_cache_keys
        .entry(prompt_cache_key.to_string())
        .and_modify(|count| *count = count.saturating_add(1))
        .or_insert(1);
}

pub(crate) async fn release_active_prompt_cache_conversation(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
    prompt_cache_key: &str,
) {
    let mut cache_state = cache.lock().await;
    let prompt_cache_key = prompt_cache_key.trim();
    let Some(count) = cache_state
        .identity_cache
        .active_prompt_cache_keys
        .get_mut(prompt_cache_key)
    else {
        return;
    };
    if *count <= 1 {
        cache_state
            .identity_cache
            .active_prompt_cache_keys
            .remove(prompt_cache_key);
    } else {
        *count -= 1;
    }
}

pub(crate) async fn release_active_prompt_cache_conversations(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
    prompt_cache_keys: &[String],
) {
    if prompt_cache_keys.is_empty() {
        return;
    }
    let mut cache_state = cache.lock().await;
    let mut released = 0_usize;
    for prompt_cache_key in prompt_cache_keys {
        let prompt_cache_key = prompt_cache_key.trim();
        let Some(count) = cache_state
            .identity_cache
            .active_prompt_cache_keys
            .get_mut(prompt_cache_key)
        else {
            continue;
        };
        if *count <= 1 {
            cache_state
                .identity_cache
                .active_prompt_cache_keys
                .remove(prompt_cache_key);
        } else {
            *count -= 1;
        }
        released += 1;
    }
    if released > 0 {
        debug!(
            released,
            candidates = prompt_cache_keys.len(),
            "released active prompt-cache conversation leases after terminal commit"
        );
    }
}

pub(crate) async fn clear_prompt_cache_conversation_identity_cache(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) {
    let mut state = cache.lock().await;
    state.identity_cache.conversations.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_encoding_is_fixed_width_and_ordered() {
        assert_eq!(
            encode_prompt_cache_conversation_sequence(0).unwrap(),
            "AAAA"
        );
        assert_eq!(
            encode_prompt_cache_conversation_sequence(1).unwrap(),
            "AAAB"
        );
        assert_eq!(
            encode_prompt_cache_conversation_sequence(30).unwrap(),
            "AAA9"
        );
        assert_eq!(
            encode_prompt_cache_conversation_sequence(31).unwrap(),
            "AABA"
        );
        assert_eq!(
            encode_prompt_cache_conversation_sequence(
                PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY - 1
            )
            .unwrap(),
            "9999"
        );
        assert!(
            encode_prompt_cache_conversation_sequence(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY)
                .is_err()
        );
    }

    #[test]
    fn invoke_id_conversation_prefix_requires_the_new_short_format() {
        assert_eq!(
            prompt_cache_conversation_id_from_invoke_id("ABCDEFAAAA"),
            Some("ABCDEF")
        );
        assert_eq!(prompt_cache_conversation_id_from_invoke_id("proxy-1"), None);
    }
}
