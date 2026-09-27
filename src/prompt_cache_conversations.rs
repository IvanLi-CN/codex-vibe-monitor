use super::*;

pub(crate) const PROMPT_CACHE_CONVERSATION_ID_LENGTH: usize = 6;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH: usize = 4;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX: u32 = 31;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY: u32 =
    PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX.pow(PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH as u32);
const PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS: usize = 5;
const PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME: &str = "prompt_cache_conversations_v1";

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationIdentity {
    pub(crate) conversation_id: String,
    pub(crate) next_sequence: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptCacheConversationIdentityCache {
    pub(crate) conversations: HashMap<String, PromptCacheConversationIdentity>,
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

pub(crate) fn prompt_cache_key_fingerprint(prompt_cache_key: &str) -> String {
    let digest = Sha256::digest(prompt_cache_key.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) async fn ensure_prompt_cache_conversations_schema(pool: &Pool<Sqlite>) -> Result<usize> {
    sqlx::query(
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
            CHECK (length(trim(prompt_cache_key)) > 0)
        )
        "#,
    )
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
        return Ok(0);
    }

    let prompt_cache_keys = sqlx::query_scalar::<_, String>(&format!(
        "SELECT DISTINCT TRIM({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL}) AS prompt_cache_key \
         FROM codex_invocations \
         WHERE {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
           AND TRIM({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL}) <> '' \
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
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(\
            SELECT 1 FROM codex_invocations \
            WHERE length(invoke_id) = ?1 AND substr(invoke_id, 1, ?2) = ?3
        )",
    )
    .bind(PROXY_INVOKE_ID_LENGTH as i64)
    .bind(PROMPT_CACHE_CONVERSATION_ID_LENGTH as i64)
    .bind(conversation_id)
    .fetch_one(pool)
    .await?
        != 0)
}

async fn create_prompt_cache_conversation_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<PromptCacheConversationIdentity> {
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let conversation_id = generate_prompt_cache_conversation_id();
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
    if let Some(prompt_cache_key) = normalize_prompt_cache_key(prompt_cache_key) {
        let identity = if let Some(identity) =
            cache.identity_cache.conversations.get_mut(prompt_cache_key)
        {
            debug!(
                prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
                conversation_id = %identity.conversation_id,
                "prompt-cache conversation identity cache hit"
            );
            identity
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
            cache
                .identity_cache
                .conversations
                .entry(prompt_cache_key.to_string())
                .or_insert(identity)
        };
        let sequence = identity.next_sequence;
        let suffix = match encode_prompt_cache_conversation_sequence(sequence) {
            Ok(suffix) => suffix,
            Err(err) => {
                error!(
                    conversation_id = %identity.conversation_id,
                    sequence,
                    error = %err,
                    "prompt-cache conversation invoke sequence exhausted"
                );
                return Err(err);
            }
        };
        identity.next_sequence = sequence.saturating_add(1);
        let invoke_id = format!("{}{}", identity.conversation_id, suffix);
        debug!(
            conversation_id = %identity.conversation_id,
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
        cache.identity_cache.unbound_prefix = Some(UnboundInvokePrefix {
            hour_key,
            prefix: generate_prompt_cache_conversation_id(),
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

pub(crate) async fn refresh_prompt_cache_conversation_stats(
    pool: &Pool<Sqlite>,
    prompt_cache_keys: &HashSet<String>,
) -> Result<usize> {
    if prompt_cache_keys.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await?;
    let mut refreshed = 0;
    for prompt_cache_key in prompt_cache_keys {
        let success_like_sql = invocation_status_is_success_like_sql("status", "error_message");
        let stats = sqlx::query_as::<_, PromptCacheConversationStatsRow>(&format!(
            r#"
            SELECT
                COUNT(*) AS request_count,
                COALESCE(SUM(CASE WHEN {success_like_sql} THEN 1 ELSE 0 END), 0) AS success_count,
                COALESCE(SUM(CASE WHEN {success_like_sql} THEN 0 ELSE 1 END), 0) AS failure_count,
                COALESCE(SUM(input_tokens), 0) AS input_tokens,
                COALESCE(SUM(output_tokens), 0) AS output_tokens,
                COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens,
                COALESCE(SUM(reported_cache_write_tokens), 0) AS reported_cache_write_tokens,
                COALESCE(SUM(reasoning_tokens), 0) AS reasoning_tokens,
                COALESCE(SUM(total_tokens), 0) AS total_tokens,
                COALESCE(SUM(CAST(cost AS REAL)), 0.0) AS cost,
                COALESCE(SUM(CAST(cost_input AS REAL)), 0.0) AS cost_input,
                COALESCE(SUM(CAST(cost_cache_write AS REAL)), 0.0) AS cost_cache_write,
                COALESCE(SUM(CAST(cost_cache_read AS REAL)), 0.0) AS cost_cache_read,
                COALESCE(SUM(CAST(cost_output AS REAL)), 0.0) AS cost_output,
                COALESCE(SUM(CAST(cost_reasoning AS REAL)), 0.0) AS cost_reasoning,
                MIN(occurred_at) AS first_invocation_at,
                MAX(occurred_at) AS last_invocation_at
            FROM codex_invocations
            WHERE TRIM({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL}) = ?1
            "#,
        ))
        .bind(prompt_cache_key)
        .fetch_one(&mut *tx)
        .await?;
        let Some(conversation_id) = sqlx::query_scalar::<_, String>(
            "SELECT conversation_id FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
        )
        .bind(prompt_cache_key)
        .fetch_optional(&mut *tx)
        .await?
        else {
            continue;
        };
        let max_sequence = sqlx::query_scalar::<_, String>(&format!(
            "SELECT invoke_id FROM codex_invocations WHERE TRIM({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL}) = ?1"
        ))
        .bind(prompt_cache_key)
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .filter_map(|invoke_id| {
            let invoke_conversation_id = prompt_cache_conversation_id_from_invoke_id(invoke_id)?;
            if invoke_conversation_id != conversation_id {
                return None;
            }
            let suffix = invoke_id_suffix(invoke_id, &conversation_id)?;
            decode_prompt_cache_conversation_sequence(suffix)
        })
        .max();
        sqlx::query(
            r#"
            UPDATE prompt_cache_conversations
            SET last_invoke_sequence = COALESCE(?1, last_invoke_sequence),
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
        .bind(stats.first_invocation_at)
        .bind(stats.last_invocation_at)
        .bind(prompt_cache_key)
        .execute(&mut *tx)
        .await?;
        refreshed += 1;
    }
    tx.commit().await?;
    debug!(
        keys = prompt_cache_keys.len(),
        refreshed, "prompt-cache conversation statistics refreshed"
    );
    Ok(refreshed)
}

fn invoke_id_suffix<'a>(invoke_id: &'a str, conversation_id: &str) -> Option<&'a str> {
    let suffix = invoke_id.strip_prefix(conversation_id)?;
    (suffix.len() == PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH).then_some(suffix)
}

pub(crate) async fn cleanup_orphan_prompt_cache_conversations(
    pool: &Pool<Sqlite>,
    dry_run: bool,
) -> Result<usize> {
    let predicate = format!(
        "TRIM({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL}) = prompt_cache_conversations.prompt_cache_key"
    );
    let count = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM prompt_cache_conversations WHERE last_invocation_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM codex_invocations WHERE {predicate})"
    ))
    .fetch_one(pool)
    .await? as usize;
    if !dry_run && count > 0 {
        sqlx::query(&format!(
            "DELETE FROM prompt_cache_conversations WHERE last_invocation_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM codex_invocations WHERE {predicate})"
        ))
        .execute(pool)
        .await?;
    }
    Ok(count)
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
