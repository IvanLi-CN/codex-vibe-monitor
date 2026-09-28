use super::*;

pub(crate) const PROMPT_CACHE_CONVERSATION_ID_LENGTH: usize = 6;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH: usize = 4;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX: u32 = 31;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY: u32 =
    PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX.pow(PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH as u32);
const PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS: usize = 5;
const PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME: &str = "prompt_cache_conversations_v1";
const PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME: &str = "prompt_cache_conversations_stats_v2";
const PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE: &str =
    "prompt_cache_conversation_stats_refresh_queue";
const PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES: i64 = 5;
const PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY: usize = 4096;
const PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE: usize = 400;
const PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_MAX_KEYS_PER_RUN: usize = 400;

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

    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} (\
            prompt_cache_key TEXT PRIMARY KEY,\
            enqueued_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache conversation statistics refresh queue")?;

    let old_prompt_cache_key_expr = invocation_prompt_cache_key_expr_sql("OLD");
    let new_prompt_cache_key_expr = invocation_prompt_cache_key_expr_sql("NEW");
    let refresh_queue_table = PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE;
    let stats_marker = PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME;
    for (trigger_name, trigger_sql) in [
        (
            "prompt_cache_conversations_stats_enqueue_insert",
            format!(
                "CREATE TRIGGER IF NOT EXISTS prompt_cache_conversations_stats_enqueue_insert \
                 AFTER INSERT ON codex_invocations \
                 WHEN {new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> '' \
                 BEGIN \
                   INSERT OR IGNORE INTO {refresh_queue_table} (prompt_cache_key) VALUES ({new_prompt_cache_key_expr}); \
                   DELETE FROM schema_refresh_migrations WHERE migration_name = '{stats_marker}'; \
                 END"
            ),
        ),
        (
            "prompt_cache_conversations_stats_enqueue_update",
            format!(
                "CREATE TRIGGER IF NOT EXISTS prompt_cache_conversations_stats_enqueue_update \
                 AFTER UPDATE OF payload, status, error_message, input_tokens, output_tokens, cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, occurred_at, invoke_id ON codex_invocations \
                 WHEN ({old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> '') \
                   OR ({new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> '') \
                 BEGIN \
                   INSERT OR IGNORE INTO {refresh_queue_table} (prompt_cache_key) SELECT {old_prompt_cache_key_expr} WHERE {old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> ''; \
                   INSERT OR IGNORE INTO {refresh_queue_table} (prompt_cache_key) SELECT {new_prompt_cache_key_expr} WHERE {new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> ''; \
                   DELETE FROM schema_refresh_migrations WHERE migration_name = '{stats_marker}'; \
                 END"
            ),
        ),
        (
            "prompt_cache_conversations_stats_enqueue_delete",
            format!(
                "CREATE TRIGGER IF NOT EXISTS prompt_cache_conversations_stats_enqueue_delete \
                 AFTER DELETE ON codex_invocations \
                 WHEN {old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> '' \
                 BEGIN \
                   INSERT OR IGNORE INTO {refresh_queue_table} (prompt_cache_key) VALUES ({old_prompt_cache_key_expr}); \
                   DELETE FROM schema_refresh_migrations WHERE migration_name = '{stats_marker}'; \
                 END"
            ),
        ),
    ] {
        sqlx::query(&trigger_sql)
            .execute(pool)
            .await
            .with_context(|| format!("failed to ensure trigger {trigger_name}"))?;
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
            let stats_refresh_already_completed = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT EXISTS(
                    SELECT 1 FROM schema_refresh_migrations WHERE migration_name = ?1
                )
                "#,
            )
            .bind(PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME)
            .fetch_one(pool)
            .await?
                != 0;
            let stats_refresh_pending = sqlx::query_scalar::<_, i64>(
                &format!(
                    "SELECT EXISTS(SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE})"
                ),
            )
            .fetch_one(pool)
            .await?
                != 0;
            if !stats_refresh_already_completed || stats_refresh_pending {
                warn!(
                    migration = PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME,
                    "prompt-cache conversation statistics need startup recovery"
                );
                let identities_created =
                    ensure_prompt_cache_conversation_rows_from_invocations(pool).await?;
                let refreshed = refresh_all_prompt_cache_conversation_stats(pool)
                    .await
                    .context("failed to recover prompt-cache conversation statistics")?;
                mark_prompt_cache_conversation_stats_fresh(pool)
                    .await
                    .context("failed to record prompt-cache conversation statistics recovery")?;
                info!(
                    migration = PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME,
                    identities_created,
                    refreshed,
                    "prompt-cache conversation statistics recovery completed"
                );
            }
            return Ok(0);
        }
        // Keep the recovery path for a manually cleared conversation table. The normal startup
        // path above is a single-row existence check instead of a full invocation reconciliation.
        warn!(
            migration = PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME,
            "prompt-cache conversation table is empty after a completed backfill; rebuilding identities"
        );
    }

    let mut backfilled = 0;
    let mut candidate_keys = 0usize;
    let mut last_prompt_cache_key = None;
    loop {
        let last_key_clause = if last_prompt_cache_key.is_some() {
            format!("AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} > ?1")
        } else {
            String::new()
        };
        let keys_sql = format!(
            "SELECT DISTINCT {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} AS prompt_cache_key \
             FROM codex_invocations \
             WHERE {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
               AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} <> '' \
               {last_key_clause} \
             ORDER BY prompt_cache_key \
             LIMIT {PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE}"
        );
        let mut keys_query = sqlx::query_scalar::<_, String>(&keys_sql);
        if let Some(last_prompt_cache_key) = last_prompt_cache_key.as_deref() {
            keys_query = keys_query.bind(last_prompt_cache_key);
        }
        let prompt_cache_keys = keys_query
            .fetch_all(pool)
            .await
            .context("failed to enumerate prompt-cache conversation backfill keys")?;
        if prompt_cache_keys.is_empty() {
            break;
        }
        candidate_keys = candidate_keys.saturating_add(prompt_cache_keys.len());
        info!(
            migration = PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME,
            candidate_keys, backfilled, "prompt-cache conversation identity backfill page started"
        );

        let mut backfill_stats_keys = HashSet::new();
        for prompt_cache_key in &prompt_cache_keys {
            backfill_stats_keys.insert(prompt_cache_key.clone());
            if ensure_prompt_cache_conversation_row(pool, prompt_cache_key).await? {
                backfilled += 1;
            }
        }
        refresh_prompt_cache_conversation_stats(pool, &backfill_stats_keys)
            .await
            .context("failed to materialize prompt-cache conversation backfill statistics")?;

        last_prompt_cache_key = prompt_cache_keys.last().cloned();
        if prompt_cache_keys.len() < PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE {
            break;
        }
    }

    sqlx::query("INSERT OR REPLACE INTO schema_refresh_migrations (migration_name) VALUES (?1)")
        .bind(PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME)
        .execute(pool)
        .await
        .context("failed to record prompt-cache conversation backfill")?;
    mark_prompt_cache_conversation_stats_fresh(pool)
        .await
        .context("failed to record prompt-cache conversation statistics backfill")?;
    info!(
        backfilled,
        "prompt-cache conversation identity backfill completed"
    );
    Ok(backfilled)
}

fn invocation_prompt_cache_key_expr_sql(alias: &str) -> String {
    format!(
        "CASE WHEN json_valid({alias}.payload) THEN TRIM(CAST(json_extract({alias}.payload, '$.promptCacheKey') AS TEXT)) END"
    )
}

async fn ensure_prompt_cache_conversation_rows_from_invocations(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let prompt_cache_key_expr = INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL;
    let mut last_prompt_cache_key = None;
    let mut created = 0;
    loop {
        let last_key_clause = if last_prompt_cache_key.is_some() {
            format!("AND {prompt_cache_key_expr} > ?1")
        } else {
            String::new()
        };
        let keys_sql = format!(
            "SELECT DISTINCT {prompt_cache_key_expr} AS prompt_cache_key \
             FROM codex_invocations \
             WHERE {prompt_cache_key_expr} IS NOT NULL \
               AND {prompt_cache_key_expr} <> '' \
               {last_key_clause} \
             ORDER BY prompt_cache_key \
             LIMIT {PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE}"
        );
        let mut keys_query = sqlx::query_scalar::<_, String>(&keys_sql);
        if let Some(last_prompt_cache_key) = last_prompt_cache_key.as_deref() {
            keys_query = keys_query.bind(last_prompt_cache_key);
        }
        let prompt_cache_keys = keys_query.fetch_all(pool).await?;
        if prompt_cache_keys.is_empty() {
            break;
        }
        for prompt_cache_key in &prompt_cache_keys {
            if ensure_prompt_cache_conversation_row(pool, prompt_cache_key).await? {
                created += 1;
            }
        }
        last_prompt_cache_key = prompt_cache_keys.last().cloned();
        if prompt_cache_keys.len() < PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE {
            break;
        }
    }
    Ok(created)
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

pub(crate) async fn ensure_prompt_cache_conversation_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<bool> {
    if load_prompt_cache_conversation_row(pool, prompt_cache_key)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    create_prompt_cache_conversation_row(pool, prompt_cache_key).await?;
    Ok(true)
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
    let alphabet = PROXY_INVOKE_ID_ALPHABET.iter().collect::<String>();
    let maximum_sequence = sqlx::query_scalar::<_, Option<i64>>(&format!(
        "SELECT MAX(\
            (instr('{alphabet}', substr(invoke_id, 7, 1)) - 1) * 29791 + \
            (instr('{alphabet}', substr(invoke_id, 8, 1)) - 1) * 961 + \
            (instr('{alphabet}', substr(invoke_id, 9, 1)) - 1) * 31 + \
            instr('{alphabet}', substr(invoke_id, 10, 1)) - 1\
         ) \
         FROM codex_invocations \
         WHERE length(invoke_id) = {PROXY_INVOKE_ID_LENGTH} \
           AND invoke_id >= ?1 \
           AND invoke_id < (?1 || '[') \
           AND instr('{alphabet}', substr(invoke_id, 7, 1)) > 0 \
           AND instr('{alphabet}', substr(invoke_id, 8, 1)) > 0 \
           AND instr('{alphabet}', substr(invoke_id, 9, 1)) > 0 \
           AND instr('{alphabet}', substr(invoke_id, 10, 1)) > 0"
    ))
    .bind(conversation_id)
    .fetch_one(pool)
    .await
    .context("failed to recover prompt-cache conversation invoke sequence")?;
    Ok(maximum_sequence.and_then(|sequence| u32::try_from(sequence).ok()))
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
    mark_prompt_cache_conversation_stats_stale(pool).await?;
    let mut last_error = None;
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_STATS_REFRESH_ATTEMPTS {
        match refresh_prompt_cache_conversation_stats_once(pool, prompt_cache_keys).await {
            Ok(refreshed) => {
                mark_prompt_cache_conversation_stats_fresh(pool).await?;
                return Ok(refreshed);
            }
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
    let prompt_cache_keys = prompt_cache_keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let mut refreshed = 0;
    for prompt_cache_keys in
        prompt_cache_keys.chunks(PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY)
    {
        let mut tx = pool.begin().await?;
        refreshed +=
            refresh_prompt_cache_conversation_stats_on_connection(tx.as_mut(), prompt_cache_keys)
                .await?;
        clear_prompt_cache_conversation_stats_refresh_queue_on_connection(
            tx.as_mut(),
            prompt_cache_keys,
        )
        .await?;
        tx.commit().await?;
    }
    debug!(
        keys = prompt_cache_keys.len(),
        refreshed, "prompt-cache conversation statistics refreshed"
    );
    Ok(refreshed)
}

pub(crate) async fn refresh_prompt_cache_conversation_stats_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_keys: &[&str],
) -> Result<usize> {
    if prompt_cache_keys.is_empty() {
        return Ok(0);
    }
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
        stats_query = stats_query.bind(*prompt_cache_key);
    }
    let stats_rows = stats_query.fetch_all(&mut *connection).await?;
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
        .execute(&mut *connection)
        .await?;
    }
    Ok(stats_rows.len())
}

pub(crate) async fn refresh_all_prompt_cache_conversation_stats(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let mut last_prompt_cache_key = None;
    let mut refreshed = 0;
    loop {
        let keys_sql = if last_prompt_cache_key.is_some() {
            format!(
                "SELECT prompt_cache_key FROM prompt_cache_conversations WHERE prompt_cache_key > ?1 ORDER BY prompt_cache_key LIMIT {PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY}"
            )
        } else {
            format!(
                "SELECT prompt_cache_key FROM prompt_cache_conversations ORDER BY prompt_cache_key LIMIT {PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY}"
            )
        };
        let mut keys_query = sqlx::query_scalar::<_, String>(&keys_sql);
        if let Some(last_prompt_cache_key) = last_prompt_cache_key.as_deref() {
            keys_query = keys_query.bind(last_prompt_cache_key);
        }
        let prompt_cache_keys = keys_query
            .fetch_all(pool)
            .await
            .context("failed to enumerate prompt-cache conversation statistics refresh keys")?;
        if prompt_cache_keys.is_empty() {
            break;
        }
        let prompt_cache_key_refs = prompt_cache_keys
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let mut tx = pool.begin().await?;
        refreshed += refresh_prompt_cache_conversation_stats_on_connection(
            tx.as_mut(),
            &prompt_cache_key_refs,
        )
        .await?;
        clear_prompt_cache_conversation_stats_refresh_queue_on_connection(
            tx.as_mut(),
            &prompt_cache_key_refs,
        )
        .await?;
        tx.commit().await?;
        last_prompt_cache_key = prompt_cache_keys.last().cloned();
        if prompt_cache_keys.len() < PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY {
            break;
        }
    }
    Ok(refreshed)
}

pub(crate) async fn clear_prompt_cache_conversation_stats_refresh_queue_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_keys: &[&str],
) -> Result<()> {
    if prompt_cache_keys.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", prompt_cache_keys.len())
        .collect::<Vec<_>>()
        .join(",");
    let delete_sql = format!(
        "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} \
         WHERE prompt_cache_key IN ({placeholders})"
    );
    let mut query = sqlx::query(&delete_sql);
    for prompt_cache_key in prompt_cache_keys {
        query = query.bind(*prompt_cache_key);
    }
    query
        .execute(&mut *connection)
        .await
        .context("failed to clear prompt-cache conversation statistics refresh queue")?;
    Ok(())
}

pub(crate) async fn mark_prompt_cache_conversation_stats_stale_on_connection(
    connection: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query("DELETE FROM schema_refresh_migrations WHERE migration_name = ?1")
        .bind(PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME)
        .execute(&mut *connection)
        .await
        .context("failed to mark prompt-cache conversation statistics stale")?;
    Ok(())
}

pub(crate) async fn mark_prompt_cache_conversation_stats_fresh_on_connection(
    connection: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO schema_refresh_migrations (migration_name) VALUES (?1)")
        .bind(PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME)
        .execute(&mut *connection)
        .await
        .context("failed to mark prompt-cache conversation statistics fresh")?;
    sqlx::query(&format!(
        "DELETE FROM schema_refresh_migrations \
         WHERE migration_name = ?1 \
           AND EXISTS (SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE})"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME)
    .execute(&mut *connection)
    .await
    .context("failed to verify prompt-cache conversation statistics freshness")?;
    Ok(())
}

pub(crate) async fn mark_prompt_cache_conversation_stats_stale(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;
    mark_prompt_cache_conversation_stats_stale_on_connection(tx.as_mut()).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn mark_prompt_cache_conversation_stats_fresh(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin().await?;
    mark_prompt_cache_conversation_stats_fresh_on_connection(tx.as_mut()).await?;
    tx.commit().await?;
    Ok(())
}

fn invoke_id_suffix<'a>(invoke_id: &'a str, conversation_id: &str) -> Option<&'a str> {
    let suffix = invoke_id.strip_prefix(conversation_id)?;
    (suffix.len() == PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH).then_some(suffix)
}

pub(crate) async fn cleanup_orphan_prompt_cache_conversations(
    pool: &Pool<Sqlite>,
    dry_run: bool,
) -> Result<usize> {
    Ok(
        cleanup_orphan_prompt_cache_conversations_with_active_keys(pool, dry_run, &HashSet::new())
            .await?
            .released,
    )
}

#[derive(Debug, Default)]
struct PromptCacheConversationOrphanCleanupResult {
    released: usize,
    deleted_prompt_cache_keys: Vec<String>,
}

async fn cleanup_orphan_prompt_cache_conversations_with_active_keys(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    active_prompt_cache_keys: &HashSet<String>,
) -> Result<PromptCacheConversationOrphanCleanupResult> {
    let predicate = format!(
        "({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} = prompt_cache_conversations.prompt_cache_key OR (length(codex_invocations.invoke_id) = {PROXY_INVOKE_ID_LENGTH} AND codex_invocations.invoke_id >= prompt_cache_conversations.conversation_id AND codex_invocations.invoke_id < (prompt_cache_conversations.conversation_id || '[')))"
    );
    let orphan_predicate = format!(
        "(last_invocation_at IS NOT NULL OR created_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now', '-{PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES} minutes')) AND updated_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now', '-{PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES} minutes') AND NOT EXISTS (SELECT 1 FROM codex_invocations WHERE {predicate})"
    );
    let mut result = PromptCacheConversationOrphanCleanupResult::default();
    let mut last_prompt_cache_key = None;
    let mut scanned_candidates = 0usize;
    loop {
        // Keep each read and delete bounded. The cache mutex is held by the mutating caller, so
        // the in-memory active-key exclusion remains stable while the predicate is rechecked.
        let last_key_clause = if last_prompt_cache_key.is_some() {
            "AND prompt_cache_key > ?1"
        } else {
            ""
        };
        let candidate_sql = format!(
            "SELECT prompt_cache_key FROM prompt_cache_conversations \
             WHERE {orphan_predicate} {last_key_clause} \
             ORDER BY prompt_cache_key \
             LIMIT {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_MAX_KEYS_PER_RUN}"
        );
        let mut candidate_query = sqlx::query_scalar::<_, String>(&candidate_sql);
        if let Some(last_prompt_cache_key) = last_prompt_cache_key.as_deref() {
            candidate_query = candidate_query.bind(last_prompt_cache_key);
        }
        let candidate_keys = candidate_query.fetch_all(pool).await?;
        if candidate_keys.is_empty() {
            break;
        }
        let candidate_key_count = candidate_keys.len();
        scanned_candidates = scanned_candidates.saturating_add(candidate_keys.len());
        last_prompt_cache_key = candidate_keys.last().cloned();
        let eligible_keys = candidate_keys
            .iter()
            .filter(|prompt_cache_key| !active_prompt_cache_keys.contains(*prompt_cache_key))
            .collect::<Vec<_>>();
        if dry_run {
            result.released = result.released.saturating_add(eligible_keys.len());
        } else if !eligible_keys.is_empty() {
            let mut tx = pool.begin().await?;
            let placeholders = std::iter::repeat_n("?", eligible_keys.len())
                .collect::<Vec<_>>()
                .join(",");
            let delete_sql = format!(
                "DELETE FROM prompt_cache_conversations WHERE {orphan_predicate} AND prompt_cache_key IN ({placeholders})"
            );
            let mut delete_query = sqlx::query(&delete_sql);
            for prompt_cache_key in &eligible_keys {
                delete_query = delete_query.bind(*prompt_cache_key);
            }
            let released = delete_query.execute(&mut *tx).await?.rows_affected() as usize;
            let remaining_placeholders = std::iter::repeat_n("?", candidate_keys.len())
                .collect::<Vec<_>>()
                .join(",");
            let remaining_sql = format!(
                "SELECT prompt_cache_key FROM prompt_cache_conversations WHERE prompt_cache_key IN ({remaining_placeholders})"
            );
            let mut remaining_query = sqlx::query_scalar::<_, String>(&remaining_sql);
            for prompt_cache_key in &candidate_keys {
                remaining_query = remaining_query.bind(prompt_cache_key);
            }
            let remaining_keys = remaining_query
                .fetch_all(&mut *tx)
                .await?
                .into_iter()
                .collect::<HashSet<_>>();
            let deleted_prompt_cache_keys = candidate_keys
                .into_iter()
                .filter(|prompt_cache_key| !remaining_keys.contains(prompt_cache_key))
                .collect::<Vec<_>>();
            if !deleted_prompt_cache_keys.is_empty() {
                let queue_placeholders = std::iter::repeat_n("?", deleted_prompt_cache_keys.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let queue_delete_sql = format!(
                    "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key IN ({queue_placeholders})"
                );
                let mut queue_delete_query = sqlx::query(&queue_delete_sql);
                for prompt_cache_key in &deleted_prompt_cache_keys {
                    queue_delete_query = queue_delete_query.bind(prompt_cache_key);
                }
                queue_delete_query.execute(&mut *tx).await?;
            }
            tx.commit().await?;
            result.released = result.released.saturating_add(released);
            result
                .deleted_prompt_cache_keys
                .extend(deleted_prompt_cache_keys);
        }
        if candidate_key_count < PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_MAX_KEYS_PER_RUN
            || scanned_candidates >= PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_MAX_KEYS_PER_RUN
        {
            break;
        }
    }
    Ok(result)
}

pub(crate) async fn cleanup_orphan_prompt_cache_conversations_with_cache(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) -> Result<usize> {
    let (active_prompt_cache_keys, cleanup) = if dry_run {
        let active_prompt_cache_keys = cache
            .lock()
            .await
            .identity_cache
            .active_prompt_cache_keys
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        let cleanup = cleanup_orphan_prompt_cache_conversations_with_active_keys(
            pool,
            true,
            &active_prompt_cache_keys,
        )
        .await?;
        (active_prompt_cache_keys, cleanup)
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
        let cleanup = cleanup_orphan_prompt_cache_conversations_with_active_keys(
            pool,
            false,
            &active_prompt_cache_keys,
        )
        .await?;
        drop(cache_state);
        (active_prompt_cache_keys, cleanup)
    };
    if !dry_run && cleanup.released > 0 {
        let mut cache_state = cache.lock().await;
        for prompt_cache_key in cleanup.deleted_prompt_cache_keys {
            cache_state
                .identity_cache
                .conversations
                .remove(&prompt_cache_key);
        }
        trim_prompt_cache_conversation_identity_cache(&mut cache_state.identity_cache);
        info!(
            released = cleanup.released,
            active = active_prompt_cache_keys.len(),
            cached = cache_state.identity_cache.conversations.len(),
            "cleared released prompt-cache conversation identities"
        );
    }
    Ok(cleanup.released)
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

#[derive(Debug)]
pub(crate) struct PromptCacheConversationLeaseDropGuard {
    cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    prompt_cache_key: Option<String>,
}

impl PromptCacheConversationLeaseDropGuard {
    pub(crate) fn new(
        cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        prompt_cache_key: Option<&str>,
    ) -> Self {
        Self {
            cache,
            prompt_cache_key: prompt_cache_key.map(str::to_owned),
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.prompt_cache_key = None;
    }
}

impl Drop for PromptCacheConversationLeaseDropGuard {
    fn drop(&mut self) {
        let Some(prompt_cache_key) = self.prompt_cache_key.take() else {
            return;
        };
        let cache = self.cache.clone();
        std::mem::drop(tokio::spawn(async move {
            release_active_prompt_cache_conversation(&cache, &prompt_cache_key).await;
        }));
    }
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
