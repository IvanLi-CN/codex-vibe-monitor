use super::*;

pub(crate) const PROMPT_CACHE_CONVERSATION_ID_LENGTH: usize = 6;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH: usize = 4;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX: u32 = 31;
pub(crate) const PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY: u32 =
    PROMPT_CACHE_CONVERSATION_SEQUENCE_RADIX.pow(PROMPT_CACHE_CONVERSATION_SEQUENCE_LENGTH as u32);
const PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS: usize = 5;
const PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME: &str = "prompt_cache_conversations_v1";
const PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME: &str = "prompt_cache_conversations_stats_v2";
pub(crate) const PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME: &str =
    "prompt_cache_conversations_materialization_v1";
const PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE: &str =
    "prompt_cache_conversation_stats_refresh_queue";
const PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE: &str =
    "prompt_cache_conversation_migration_progress";
const PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE: &str =
    "prompt_cache_conversation_materialization_runs";
const PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL: &str = "identity_backfill";
const PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION: &str = "identity_reconciliation";
const PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD: &str = "stats_rebuild";
const PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN: &str = "queue_drain";
const PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE: &str = "complete";
const PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES: i64 = 5;
const PROMPT_CACHE_CONVERSATION_IDENTITY_CACHE_CAPACITY: usize = 4096;
const PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE: usize = 400;
const PROMPT_CACHE_CONVERSATION_BATCH_INITIAL_SIZE: usize = 64;
const PROMPT_CACHE_CONVERSATION_BATCH_MIN_SIZE: usize = 32;
const PROMPT_CACHE_CONVERSATION_BATCH_MAX_SIZE: usize =
    PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE;
const PROMPT_CACHE_CONVERSATION_BATCH_LOW_LATENCY_MS: u128 = 50;
const PROMPT_CACHE_CONVERSATION_BATCH_HIGH_LATENCY_MS: u128 = 200;
const PROMPT_CACHE_CONVERSATION_BATCH_BOUNDARY_PAUSE: Duration = Duration::from_millis(10);
const PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_MAX_KEYS_PER_RUN: usize = 400;

static PROMPT_CACHE_CONVERSATION_BATCH_CONTROLLER: Lazy<
    std::sync::Mutex<PromptCacheConversationBatchController>,
> = Lazy::new(|| std::sync::Mutex::new(PromptCacheConversationBatchController::default()));

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

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PromptCacheConversationMigrationProgressRow {
    pub(crate) phase: String,
    source_max_invocation_id: i64,
    cursor_key: Option<String>,
    updated_at: String,
    total_keys: Option<i64>,
    pub(crate) completed_keys: Option<i64>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationMaterializationRunRecord {
    pub(crate) id: i64,
    pub(crate) started_at: String,
    pub(crate) finished_at: String,
    pub(crate) phase: String,
    pub(crate) status: String,
    pub(crate) scanned: i64,
    pub(crate) updated: i64,
    pub(crate) batch_count: i64,
    pub(crate) last_batch_size: i64,
    pub(crate) max_batch_size: i64,
    pub(crate) batch_elapsed_ms: i64,
    pub(crate) duration_ms: i64,
    pub(crate) defer_reason: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationMaterializationStatus {
    pub(crate) enabled: bool,
    pub(crate) phase: String,
    pub(crate) total_keys: Option<u64>,
    pub(crate) completed_keys: u64,
    pub(crate) queue_pending: u64,
    pub(crate) progress_percent: Option<f64>,
    pub(crate) estimated_remaining_ms: Option<u64>,
    pub(crate) source_max_invocation_id: u64,
    pub(crate) updated_at: String,
    pub(crate) last_started_at: Option<String>,
    pub(crate) last_finished_at: Option<String>,
    pub(crate) last_status: String,
    pub(crate) suspension_reason: Option<String>,
    pub(crate) recent_runs: Vec<PromptCacheConversationMaterializationRunRecord>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptCacheConversationMaterializationRun {
    pub(crate) phase: String,
    pub(crate) scanned: u64,
    pub(crate) updated: u64,
    pub(crate) hit_scan_limit: bool,
    pub(crate) complete: bool,
    pub(crate) page_complete: bool,
    pub(crate) deferred: bool,
    pub(crate) defer_reason: Option<&'static str>,
    pub(crate) batch_count: u64,
    pub(crate) last_batch_size: usize,
    pub(crate) max_batch_size: usize,
    pub(crate) batch_elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptCacheConversationBatchPolicy {
    Adaptive,
    Fixed(usize),
}

#[derive(Debug, Clone, Copy)]
struct PromptCacheConversationBatchController {
    batch_size: usize,
    low_latency_successes: u8,
}

impl Default for PromptCacheConversationBatchController {
    fn default() -> Self {
        Self {
            batch_size: PROMPT_CACHE_CONVERSATION_BATCH_INITIAL_SIZE,
            low_latency_successes: 0,
        }
    }
}

impl PromptCacheConversationBatchController {
    fn for_policy(policy: PromptCacheConversationBatchPolicy) -> Self {
        match policy {
            PromptCacheConversationBatchPolicy::Adaptive => {
                PROMPT_CACHE_CONVERSATION_BATCH_CONTROLLER
                    .lock()
                    .expect("prompt-cache batch controller mutex")
                    .to_owned()
            }
            PromptCacheConversationBatchPolicy::Fixed(batch_size) => Self {
                batch_size: batch_size.clamp(
                    PROMPT_CACHE_CONVERSATION_BATCH_MIN_SIZE,
                    PROMPT_CACHE_CONVERSATION_BATCH_MAX_SIZE,
                ),
                low_latency_successes: 0,
            },
        }
    }

    fn persist_adaptive(self, policy: PromptCacheConversationBatchPolicy) {
        if policy == PromptCacheConversationBatchPolicy::Adaptive {
            *PROMPT_CACHE_CONVERSATION_BATCH_CONTROLLER
                .lock()
                .expect("prompt-cache batch controller mutex") = self;
        }
    }

    fn next_batch_size(self, remaining: usize) -> usize {
        self.batch_size.min(remaining).max(1)
    }

    fn observe_success(&mut self, elapsed_ms: u128, pressure_waiter: bool) {
        if pressure_waiter || elapsed_ms >= PROMPT_CACHE_CONVERSATION_BATCH_HIGH_LATENCY_MS {
            self.batch_size = (self.batch_size / 2).max(PROMPT_CACHE_CONVERSATION_BATCH_MIN_SIZE);
            self.low_latency_successes = 0;
        } else if elapsed_ms <= PROMPT_CACHE_CONVERSATION_BATCH_LOW_LATENCY_MS {
            self.low_latency_successes = self.low_latency_successes.saturating_add(1);
            if self.low_latency_successes >= 2 {
                self.batch_size = self
                    .batch_size
                    .saturating_mul(2)
                    .min(PROMPT_CACHE_CONVERSATION_BATCH_MAX_SIZE);
                self.low_latency_successes = 0;
            }
        } else {
            self.low_latency_successes = 0;
        }
    }

    fn observe_failure(&mut self) {
        self.batch_size = (self.batch_size / 2).max(PROMPT_CACHE_CONVERSATION_BATCH_MIN_SIZE);
        self.low_latency_successes = 0;
    }
}

struct PromptCacheConversationMaterializationContext<'a> {
    pool: &'a Pool<Sqlite>,
    page_limit: usize,
    started_at: Instant,
    max_elapsed: Option<Duration>,
    policy: PromptCacheConversationBatchPolicy,
    controller: &'a mut PromptCacheConversationBatchController,
    should_yield: &'a (dyn Fn() -> bool + Send + Sync),
    check_operator_enabled: bool,
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

pub(crate) async fn ensure_prompt_cache_conversations_schema(pool: &Pool<Sqlite>) -> Result<()> {
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

    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} (\
            migration_name TEXT PRIMARY KEY,\
            phase TEXT NOT NULL,\
            source_max_invocation_id INTEGER NOT NULL DEFAULT 0,\
            cursor_key TEXT,\
            completed_keys INTEGER NOT NULL DEFAULT 0,\
            updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache conversation migration progress table")?;
    ensure_prompt_cache_column(pool, "total_keys", "INTEGER").await?;
    ensure_prompt_cache_column(pool, "completed_keys", "INTEGER NOT NULL DEFAULT 0").await?;
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE} (\
            id INTEGER PRIMARY KEY AUTOINCREMENT,\
            started_at TEXT NOT NULL,\
            finished_at TEXT NOT NULL,\
            phase TEXT NOT NULL,\
            status TEXT NOT NULL,\
            scanned INTEGER NOT NULL DEFAULT 0,\
            updated INTEGER NOT NULL DEFAULT 0,\
            batch_count INTEGER NOT NULL DEFAULT 0,\
            last_batch_size INTEGER NOT NULL DEFAULT 0,\
            max_batch_size INTEGER NOT NULL DEFAULT 0,\
            batch_elapsed_ms INTEGER NOT NULL DEFAULT 0,\
            duration_ms INTEGER NOT NULL DEFAULT 0,\
            defer_reason TEXT,\
            error TEXT\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache materialization run history table")?;
    sqlx::query(&format!(
        "CREATE INDEX IF NOT EXISTS idx_prompt_cache_materialization_runs_started_at \
         ON {PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE} (started_at DESC, id DESC)"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache materialization run history index")?;
    sqlx::query(&format!(
        "INSERT OR IGNORE INTO {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         (migration_name, phase, source_max_invocation_id, cursor_key) VALUES (?1, ?2, 0, NULL)"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .bind(PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL)
    .execute(pool)
    .await
    .context("failed to initialize prompt-cache conversation migration progress")?;
    repair_prompt_cache_conversation_progress_counters(pool).await?;
    Ok(())
}

async fn ensure_prompt_cache_column(
    pool: &Pool<Sqlite>,
    column_name: &str,
    definition: &str,
) -> Result<()> {
    let columns = sqlx::query("PRAGMA table_info(prompt_cache_conversation_migration_progress)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    if columns.contains(column_name) {
        return Ok(());
    }
    let statement = format!(
        "ALTER TABLE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         ADD COLUMN {column_name} {definition}"
    );
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

fn invocation_prompt_cache_key_expr_sql(alias: &str) -> String {
    format!(
        "CASE WHEN json_valid({alias}.payload) THEN TRIM(CAST(json_extract({alias}.payload, '$.promptCacheKey') AS TEXT)) END"
    )
}

async fn load_prompt_cache_conversation_migration_progress_on_connection(
    connection: &mut SqliteConnection,
) -> Result<PromptCacheConversationMigrationProgressRow> {
    sqlx::query_as::<_, PromptCacheConversationMigrationProgressRow>(&format!(
        "SELECT phase, source_max_invocation_id, cursor_key, updated_at, total_keys, completed_keys \
         FROM {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         WHERE migration_name = ?1"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| {
        anyhow!(
            "prompt-cache conversation migration progress row is missing: {}",
            PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME
        )
    })
}

pub(crate) async fn load_prompt_cache_conversation_migration_progress(
    pool: &Pool<Sqlite>,
) -> Result<PromptCacheConversationMigrationProgressRow> {
    let mut connection = pool.acquire().await?;
    load_prompt_cache_conversation_migration_progress_on_connection(&mut connection).await
}

async fn ensure_prompt_cache_conversation_total_keys(
    pool: &Pool<Sqlite>,
    source_max_invocation_id: i64,
) -> Result<()> {
    let existing = sqlx::query_scalar::<_, Option<i64>>(&format!(
        r#"
        SELECT total_keys
        FROM {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE}
        WHERE migration_name = ?1
        "#
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .fetch_one(pool)
    .await?;
    if existing.is_some() {
        return Ok(());
    }

    let invocation_key_expr = invocation_prompt_cache_key_expr_sql("i");
    let total = sqlx::query_scalar::<_, i64>(&format!(
        r#"
        SELECT COUNT(*)
        FROM (
            SELECT DISTINCT {invocation_key_expr}
            FROM codex_invocations AS i
            WHERE i.id <= ?1
              AND {invocation_key_expr} IS NOT NULL
              AND {invocation_key_expr} <> ''
        )
        "#
    ))
    .bind(source_max_invocation_id)
    .fetch_one(pool)
    .await?;
    sqlx::query(&format!(
        r#"
        UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE}
        SET total_keys = ?1,
            updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE migration_name = ?2
        "#
    ))
    .bind(total)
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .execute(pool)
    .await?;
    Ok(())
}

async fn repair_prompt_cache_conversation_progress_counters(pool: &Pool<Sqlite>) -> Result<()> {
    let phase = sqlx::query_scalar::<_, String>(&format!(
        "SELECT phase FROM {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         WHERE migration_name = ?1"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .fetch_one(pool)
    .await?;
    let needs_total = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         WHERE migration_name = ?1 AND total_keys IS NULL)"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .fetch_one(pool)
    .await?
        != 0;
    if needs_total && phase == PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE {
        sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
             SET total_keys = (SELECT COUNT(*) FROM prompt_cache_conversations), \
                 updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE migration_name = ?1"
        ))
        .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
        .execute(pool)
        .await?;
    }
    if phase == PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE {
        sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
             SET completed_keys = COALESCE(total_keys, completed_keys), \
                 updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE migration_name = ?1"
        ))
        .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
        .execute(pool)
        .await?;
    } else if phase == PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD {
        sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
             SET completed_keys = 0, updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE migration_name = ?1 AND total_keys IS NOT NULL \
               AND completed_keys >= total_keys"
        ))
        .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub(crate) async fn record_prompt_cache_conversation_materialization_run(
    pool: &Pool<Sqlite>,
    started_at: &str,
    duration_ms: u64,
    outcome: &PromptCacheConversationMaterializationRun,
    status: &str,
    error: Option<&str>,
) -> Result<()> {
    let to_i64 = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let error = error.map(|value| value.chars().take(512).collect::<String>());
    sqlx::query(&format!(
        r#"
        INSERT INTO {PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE} (
            started_at, finished_at, phase, status, scanned, updated, batch_count,
            last_batch_size, max_batch_size, batch_elapsed_ms, duration_ms, defer_reason, error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#
    ))
    .bind(started_at)
    .bind(format_utc_iso(Utc::now()))
    .bind(&outcome.phase)
    .bind(status)
    .bind(to_i64(outcome.scanned))
    .bind(to_i64(outcome.updated))
    .bind(to_i64(outcome.batch_count))
    .bind(to_i64(outcome.last_batch_size as u64))
    .bind(to_i64(outcome.max_batch_size as u64))
    .bind(to_i64(outcome.batch_elapsed_ms))
    .bind(to_i64(duration_ms))
    .bind(outcome.defer_reason)
    .bind(error)
    .execute(pool)
    .await?;
    sqlx::query(&format!(
        r#"
        DELETE FROM {PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE}
        WHERE id NOT IN (
            SELECT id
            FROM {PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE}
            ORDER BY id DESC
            LIMIT 100
        )
        "#
    ))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn load_prompt_cache_conversation_materialization_status(
    pool: &Pool<Sqlite>,
) -> Result<PromptCacheConversationMaterializationStatus> {
    let progress = load_prompt_cache_conversation_migration_progress(pool).await?;
    let task = crate::load_startup_backfill_progress(
        pool,
        PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME,
    )
    .await?;
    let queue_pending = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM (SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} LIMIT 1001)"
    ))
    .fetch_one(pool)
    .await?
    .max(0) as u64;
    let materialization_complete =
        prompt_cache_conversation_materialization_is_complete(pool).await?;
    let total_keys = progress.total_keys.map(|value| value.max(0) as u64);
    let completed_keys = progress.completed_keys.unwrap_or(0).max(0) as u64;
    let completed_keys = total_keys
        .map(|total| completed_keys.min(total))
        .unwrap_or(completed_keys);
    let progress_percent = total_keys.and_then(|total| {
        (total > 0).then(|| {
            let percent = (completed_keys as f64 / total as f64 * 100.0).min(100.0);
            if materialization_complete {
                percent
            } else {
                percent.min(99.0)
            }
        })
    });
    let recent_runs = sqlx::query_as::<_, PromptCacheConversationMaterializationRunRecord>(
        &format!(
            r#"
            SELECT id, started_at, finished_at, phase, status, scanned, updated, batch_count,
                   last_batch_size, max_batch_size, batch_elapsed_ms, duration_ms, defer_reason, error
            FROM {PROMPT_CACHE_CONVERSATIONS_RUNS_TABLE}
            ORDER BY id DESC
            LIMIT 10
            "#
        ),
    )
    .fetch_all(pool)
    .await?;
    let (sampled_ms, sampled_keys) = recent_runs
        .iter()
        .filter(|run| run.status != "failed" && run.scanned > 0 && run.duration_ms >= 0)
        .fold((0_u64, 0_u64), |(ms, keys), run| {
            (
                ms.saturating_add(run.duration_ms.max(0) as u64),
                keys.saturating_add(run.scanned.max(0) as u64),
            )
        });
    let estimated_remaining_ms = total_keys.and_then(|total| {
        let remaining = total.saturating_sub(completed_keys);
        if remaining == 0 && materialization_complete {
            return Some(0);
        }
        if sampled_keys == 0 || remaining == 0 {
            return None;
        }
        Some(
            ((remaining as f64 * sampled_ms as f64 / sampled_keys as f64).ceil() as u64)
                .min(365 * 24 * 60 * 60 * 1000),
        )
    });

    Ok(PromptCacheConversationMaterializationStatus {
        enabled: task.enabled,
        phase: progress.phase,
        total_keys,
        completed_keys,
        queue_pending,
        progress_percent,
        estimated_remaining_ms,
        source_max_invocation_id: progress.source_max_invocation_id.max(0) as u64,
        updated_at: progress.updated_at,
        last_started_at: task.last_started_at,
        last_finished_at: task.last_finished_at,
        last_status: if task.enabled {
            task.last_status
        } else {
            "disabled".to_string()
        },
        suspension_reason: task.suspension_reason,
        recent_runs,
    })
}

async fn update_prompt_cache_conversation_migration_progress_on_connection(
    connection: &mut SqliteConnection,
    phase: &str,
    source_max_invocation_id: i64,
    cursor_key: Option<&str>,
) -> Result<()> {
    sqlx::query(&format!(
        "UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         SET phase = ?1, source_max_invocation_id = ?2, cursor_key = ?3, \
             updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE migration_name = ?4"
    ))
    .bind(phase)
    .bind(source_max_invocation_id)
    .bind(cursor_key)
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .execute(&mut *connection)
    .await
    .context("failed to update prompt-cache conversation migration progress")?;
    Ok(())
}

async fn update_prompt_cache_conversation_migration_progress(
    pool: &Pool<Sqlite>,
    phase: &str,
    source_max_invocation_id: i64,
    cursor_key: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    update_prompt_cache_conversation_migration_progress_on_connection(
        tx.as_mut(),
        phase,
        source_max_invocation_id,
        cursor_key,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn increment_prompt_cache_conversation_completed_keys_on_connection(
    connection: &mut SqliteConnection,
    completed_keys: usize,
) -> Result<()> {
    sqlx::query(&format!(
        "UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         SET completed_keys = COALESCE(completed_keys, 0) + ?1, \
             updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE migration_name = ?2"
    ))
    .bind(i64::try_from(completed_keys).unwrap_or(i64::MAX))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn mark_prompt_cache_conversation_completed_keys_on_connection(
    connection: &mut SqliteConnection,
) -> Result<()> {
    sqlx::query(&format!(
        "UPDATE {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} \
         SET completed_keys = COALESCE(total_keys, completed_keys), \
             updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE migration_name = ?1"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

async fn prompt_cache_conversation_materialization_enabled(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT enabled FROM startup_backfill_progress WHERE task_name = ?1",
    )
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .fetch_optional(pool)
    .await?
    .is_none_or(|enabled| enabled != 0))
}

async fn prompt_cache_conversation_marker_exists_on_connection(
    connection: &mut SqliteConnection,
    migration_name: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM schema_refresh_migrations WHERE migration_name = ?1)",
    )
    .bind(migration_name)
    .fetch_one(&mut *connection)
    .await?
        != 0)
}

async fn prompt_cache_conversation_marker_exists(
    pool: &Pool<Sqlite>,
    migration_name: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM schema_refresh_migrations WHERE migration_name = ?1)",
    )
    .bind(migration_name)
    .fetch_one(pool)
    .await?
        != 0)
}

async fn prompt_cache_conversation_refresh_queue_has_rows_on_connection(
    connection: &mut SqliteConnection,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE})"
    ))
    .fetch_one(&mut *connection)
    .await?
        != 0)
}

pub(crate) async fn prompt_cache_conversation_materialization_is_complete_on_connection(
    connection: &mut SqliteConnection,
) -> Result<bool> {
    let progress =
        load_prompt_cache_conversation_migration_progress_on_connection(connection).await?;
    if progress.phase != PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE {
        return Ok(false);
    }
    if !prompt_cache_conversation_marker_exists_on_connection(
        connection,
        PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME,
    )
    .await?
    {
        return Ok(false);
    }
    if !prompt_cache_conversation_marker_exists_on_connection(
        connection,
        PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME,
    )
    .await?
    {
        return Ok(false);
    }
    Ok(!prompt_cache_conversation_refresh_queue_has_rows_on_connection(connection).await?)
}

pub(crate) async fn prompt_cache_conversation_materialization_is_complete(
    pool: &Pool<Sqlite>,
) -> Result<bool> {
    let mut transaction = pool.begin().await?;
    let complete =
        prompt_cache_conversation_materialization_is_complete_on_connection(transaction.as_mut())
            .await?;
    transaction.commit().await?;
    Ok(complete)
}

fn prompt_cache_conversation_materialization_budget_exhausted(
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> bool {
    max_elapsed.is_some_and(|budget| started_at.elapsed() >= budget)
}

async fn prompt_cache_conversation_invocation_snapshot_max_id(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(pool)
            .await?
            .unwrap_or_default(),
    )
}

async fn prompt_cache_conversation_identity_repair_needed(pool: &Pool<Sqlite>) -> Result<bool> {
    let invocation_key_expr = invocation_prompt_cache_key_expr_sql("i");
    Ok(sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(\
            SELECT 1 FROM codex_invocations AS i \
            WHERE {invocation_key_expr} IS NOT NULL \
              AND {invocation_key_expr} <> '' \
              AND NOT EXISTS (\
                  SELECT 1 FROM prompt_cache_conversations AS c \
                  WHERE c.prompt_cache_key = {invocation_key_expr}\
              )\
        )"
    ))
    .fetch_one(pool)
    .await?
        != 0)
}

async fn prompt_cache_conversation_materialize_key_batch(
    pool: &Pool<Sqlite>,
    phase: &str,
    source_max_invocation_id: i64,
    advance_cursor: bool,
    prompt_cache_keys: &[String],
) -> Result<(usize, usize, Duration)> {
    let started_at = Instant::now();
    let mut tx = pool.begin().await?;
    let existing_keys =
        load_prompt_cache_conversation_keys_on_connection(tx.as_mut(), prompt_cache_keys).await?;
    let mut identities_created = 0;
    for prompt_cache_key in prompt_cache_keys {
        if !existing_keys.contains(prompt_cache_key) {
            let (_, created) =
                create_prompt_cache_conversation_row_on_connection(tx.as_mut(), prompt_cache_key)
                    .await?;
            if created {
                identities_created += 1;
            }
        }
    }
    let prompt_cache_key_refs = prompt_cache_keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let refreshed =
        refresh_prompt_cache_conversation_stats_on_connection(tx.as_mut(), &prompt_cache_key_refs)
            .await?;
    clear_prompt_cache_conversation_stats_refresh_queue_on_connection(
        tx.as_mut(),
        &prompt_cache_key_refs,
    )
    .await?;
    if advance_cursor {
        update_prompt_cache_conversation_migration_progress_on_connection(
            tx.as_mut(),
            phase,
            source_max_invocation_id,
            prompt_cache_keys.last().map(String::as_str),
        )
        .await?;
        if phase != PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD {
            increment_prompt_cache_conversation_completed_keys_on_connection(
                tx.as_mut(),
                prompt_cache_keys.len(),
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok((identities_created, refreshed, started_at.elapsed()))
}

async fn load_prompt_cache_conversation_keys_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_keys: &[String],
) -> Result<HashSet<String>> {
    if prompt_cache_keys.is_empty() {
        return Ok(HashSet::new());
    }
    let placeholders = std::iter::repeat_n("?", prompt_cache_keys.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT prompt_cache_key FROM prompt_cache_conversations WHERE prompt_cache_key IN ({placeholders})"
    );
    let mut query = sqlx::query_scalar::<_, String>(&sql);
    for prompt_cache_key in prompt_cache_keys {
        query = query.bind(prompt_cache_key);
    }
    Ok(query
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .collect())
}

async fn run_prompt_cache_conversation_adaptive_key_batches(
    context: &mut PromptCacheConversationMaterializationContext<'_>,
    phase: &str,
    source_max_invocation_id: i64,
    advance_cursor: bool,
    prompt_cache_keys: &[String],
) -> Result<PromptCacheConversationMaterializationRun> {
    let mut result = PromptCacheConversationMaterializationRun {
        phase: phase.to_string(),
        hit_scan_limit: true,
        ..Default::default()
    };
    let mut offset = 0;
    while offset < prompt_cache_keys.len() {
        if prompt_cache_conversation_materialization_budget_exhausted(
            context.started_at,
            context.max_elapsed,
        ) {
            return Ok(result);
        }
        if context.check_operator_enabled
            && !prompt_cache_conversation_materialization_enabled(context.pool).await?
        {
            result.deferred = true;
            result.defer_reason = Some("operator_disabled");
            return Ok(result);
        }
        if (context.should_yield)() {
            result.deferred = true;
            result.defer_reason = Some("coordinator_priority");
            context.controller.persist_adaptive(context.policy);
            return Ok(result);
        }

        let batch_size = context
            .controller
            .next_batch_size(prompt_cache_keys.len() - offset);
        let batch_end = (offset + batch_size).min(prompt_cache_keys.len());
        let batch = &prompt_cache_keys[offset..batch_end];
        let (identities_created, refreshed, elapsed) =
            match prompt_cache_conversation_materialize_key_batch(
                context.pool,
                phase,
                source_max_invocation_id,
                advance_cursor,
                batch,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    context.controller.observe_failure();
                    context.controller.persist_adaptive(context.policy);
                    return Err(error);
                }
            };
        let priority_waiter = (context.should_yield)();
        context
            .controller
            .observe_success(elapsed.as_millis(), priority_waiter);
        context.controller.persist_adaptive(context.policy);

        offset = batch_end;
        result.batch_count = result.batch_count.saturating_add(1);
        result.last_batch_size = batch.len();
        result.max_batch_size = result.max_batch_size.max(batch.len());
        result.batch_elapsed_ms = result
            .batch_elapsed_ms
            .saturating_add(elapsed.as_millis().min(u128::from(u64::MAX)) as u64);
        result.scanned = result.scanned.saturating_add(batch.len() as u64);
        result.updated = result
            .updated
            .saturating_add(identities_created.saturating_add(refreshed) as u64);

        if context.check_operator_enabled
            && !prompt_cache_conversation_materialization_enabled(context.pool).await?
        {
            result.deferred = true;
            result.defer_reason = Some("operator_disabled");
            return Ok(result);
        }

        if priority_waiter {
            result.deferred = true;
            result.defer_reason = Some("coordinator_priority");
            return Ok(result);
        }
        if context.policy == PromptCacheConversationBatchPolicy::Adaptive
            && offset < prompt_cache_keys.len()
        {
            // Give interactive readers a scheduler window between adaptive commits. The
            // transaction above has already committed, so this cannot interrupt a micro-batch.
            tokio::time::sleep(PROMPT_CACHE_CONVERSATION_BATCH_BOUNDARY_PAUSE).await;
        }
    }
    result.page_complete = true;
    Ok(result)
}

async fn run_prompt_cache_conversation_adaptive_identity_backfill_page(
    context: &mut PromptCacheConversationMaterializationContext<'_>,
    progress: &PromptCacheConversationMigrationProgressRow,
) -> Result<PromptCacheConversationMaterializationRun> {
    let source_max_invocation_id = if progress.source_max_invocation_id == 0 {
        prompt_cache_conversation_invocation_snapshot_max_id(context.pool).await?
    } else {
        progress.source_max_invocation_id
    };
    if source_max_invocation_id != progress.source_max_invocation_id {
        update_prompt_cache_conversation_migration_progress(
            context.pool,
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
            source_max_invocation_id,
            progress.cursor_key.as_deref(),
        )
        .await?;
    }
    ensure_prompt_cache_conversation_total_keys(context.pool, source_max_invocation_id).await?;

    let (last_key_clause, limit_placeholder) = progress
        .cursor_key
        .as_deref()
        .map(|_| {
            (
                format!("AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} > ?2"),
                "?3",
            )
        })
        .unwrap_or_else(|| (String::new(), "?2"));
    let keys_sql = format!(
        "SELECT DISTINCT {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} AS prompt_cache_key \
         FROM codex_invocations \
         WHERE id <= ?1 \
           AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} IS NOT NULL \
           AND {INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL} <> '' \
           {last_key_clause} \
         ORDER BY prompt_cache_key \
         LIMIT {limit_placeholder}"
    );
    let mut keys_query = sqlx::query_scalar::<_, String>(&keys_sql).bind(source_max_invocation_id);
    if let Some(cursor_key) = progress.cursor_key.as_deref() {
        keys_query = keys_query.bind(cursor_key);
    }
    let prompt_cache_keys = keys_query
        .bind(context.page_limit as i64)
        .fetch_all(context.pool)
        .await?;
    if prompt_cache_keys.is_empty() {
        update_prompt_cache_conversation_migration_progress(
            context.pool,
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION,
            source_max_invocation_id,
            None,
        )
        .await?;
        return Ok(PromptCacheConversationMaterializationRun {
            phase: PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION.to_string(),
            updated: 1,
            hit_scan_limit: true,
            page_complete: true,
            ..Default::default()
        });
    }

    let mut result = run_prompt_cache_conversation_adaptive_key_batches(
        context,
        PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
        source_max_invocation_id,
        true,
        &prompt_cache_keys,
    )
    .await?;
    if !result.page_complete || result.deferred {
        return Ok(result);
    }
    let next_phase = if prompt_cache_keys.len() < context.page_limit {
        PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION
    } else {
        PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL
    };
    let next_cursor =
        (next_phase == PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL).then(|| {
            prompt_cache_keys
                .last()
                .expect("non-empty key page")
                .as_str()
        });
    update_prompt_cache_conversation_migration_progress(
        context.pool,
        next_phase,
        source_max_invocation_id,
        next_cursor,
    )
    .await?;
    result.phase = next_phase.to_string();
    Ok(result)
}

async fn run_prompt_cache_conversation_adaptive_identity_reconciliation_page(
    context: &mut PromptCacheConversationMaterializationContext<'_>,
    progress: &PromptCacheConversationMigrationProgressRow,
) -> Result<PromptCacheConversationMaterializationRun> {
    let invocation_key_expr = invocation_prompt_cache_key_expr_sql("i");
    let keys_sql = format!(
        "SELECT DISTINCT {invocation_key_expr} AS prompt_cache_key \
         FROM codex_invocations AS i \
         WHERE {invocation_key_expr} IS NOT NULL \
           AND {invocation_key_expr} <> '' \
           AND NOT EXISTS(\
               SELECT 1 FROM prompt_cache_conversations AS c \
               WHERE c.prompt_cache_key = {invocation_key_expr}\
           ) \
         ORDER BY prompt_cache_key \
         LIMIT ?1"
    );
    let prompt_cache_keys = sqlx::query_scalar::<_, String>(&keys_sql)
        .bind(context.page_limit as i64)
        .fetch_all(context.pool)
        .await?;
    if prompt_cache_keys.is_empty() {
        let mut tx = context.pool.begin().await?;
        sqlx::query(
            "INSERT OR REPLACE INTO schema_refresh_migrations (migration_name) VALUES (?1)",
        )
        .bind(PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME)
        .execute(tx.as_mut())
        .await
        .context("failed to record prompt-cache conversation identity backfill")?;
        update_prompt_cache_conversation_migration_progress_on_connection(
            tx.as_mut(),
            PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD,
            progress.source_max_invocation_id,
            None,
        )
        .await?;
        tx.commit().await?;
        return Ok(PromptCacheConversationMaterializationRun {
            phase: PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD.to_string(),
            updated: 1,
            hit_scan_limit: true,
            page_complete: true,
            ..Default::default()
        });
    }

    run_prompt_cache_conversation_adaptive_key_batches(
        context,
        PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION,
        progress.source_max_invocation_id,
        false,
        &prompt_cache_keys,
    )
    .await
}

async fn run_prompt_cache_conversation_adaptive_stats_rebuild_page(
    context: &mut PromptCacheConversationMaterializationContext<'_>,
    progress: &PromptCacheConversationMigrationProgressRow,
) -> Result<PromptCacheConversationMaterializationRun> {
    let (last_key_clause, limit_placeholder) = progress
        .cursor_key
        .as_deref()
        .map(|_| ("WHERE prompt_cache_key > ?1", "?2"))
        .unwrap_or(("", "?1"));
    let keys_sql = format!(
        "SELECT prompt_cache_key FROM prompt_cache_conversations \
         {last_key_clause} ORDER BY prompt_cache_key LIMIT {limit_placeholder}"
    );
    let mut keys_query = sqlx::query_scalar::<_, String>(&keys_sql);
    if let Some(cursor_key) = progress.cursor_key.as_deref() {
        keys_query = keys_query.bind(cursor_key);
    }
    let prompt_cache_keys = keys_query
        .bind(context.page_limit as i64)
        .fetch_all(context.pool)
        .await?;
    if prompt_cache_keys.is_empty() {
        update_prompt_cache_conversation_migration_progress(
            context.pool,
            PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN,
            progress.source_max_invocation_id,
            None,
        )
        .await?;
        return Ok(PromptCacheConversationMaterializationRun {
            phase: PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN.to_string(),
            updated: 1,
            hit_scan_limit: true,
            page_complete: true,
            ..Default::default()
        });
    }

    let mut result = run_prompt_cache_conversation_adaptive_key_batches(
        context,
        PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD,
        progress.source_max_invocation_id,
        true,
        &prompt_cache_keys,
    )
    .await?;
    if !result.page_complete || result.deferred {
        return Ok(result);
    }
    let next_phase = if prompt_cache_keys.len() < context.page_limit {
        PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN
    } else {
        PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD
    };
    let next_cursor = (next_phase == PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD).then(|| {
        prompt_cache_keys
            .last()
            .expect("non-empty key page")
            .as_str()
    });
    update_prompt_cache_conversation_migration_progress(
        context.pool,
        next_phase,
        progress.source_max_invocation_id,
        next_cursor,
    )
    .await?;
    result.phase = next_phase.to_string();
    Ok(result)
}

async fn run_prompt_cache_conversation_adaptive_queue_drain_page(
    context: &mut PromptCacheConversationMaterializationContext<'_>,
    progress: &PromptCacheConversationMigrationProgressRow,
) -> Result<PromptCacheConversationMaterializationRun> {
    let prompt_cache_keys = sqlx::query_scalar::<_, String>(&format!(
        "SELECT prompt_cache_key FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} \
         ORDER BY prompt_cache_key LIMIT ?1"
    ))
    .bind(context.page_limit as i64)
    .fetch_all(context.pool)
    .await?;
    if prompt_cache_keys.is_empty() {
        if (context.should_yield)() {
            return Ok(PromptCacheConversationMaterializationRun {
                phase: PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN.to_string(),
                hit_scan_limit: true,
                deferred: true,
                defer_reason: Some("coordinator_priority"),
                ..Default::default()
            });
        }
        let mut tx = context.pool.begin().await?;
        let identity_complete = prompt_cache_conversation_marker_exists_on_connection(
            tx.as_mut(),
            PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME,
        )
        .await?;
        let queue_pending =
            prompt_cache_conversation_refresh_queue_has_rows_on_connection(tx.as_mut()).await?;
        if identity_complete && !queue_pending {
            update_prompt_cache_conversation_migration_progress_on_connection(
                tx.as_mut(),
                PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE,
                progress.source_max_invocation_id,
                None,
            )
            .await?;
            mark_prompt_cache_conversation_completed_keys_on_connection(tx.as_mut()).await?;
            mark_prompt_cache_conversation_stats_fresh_on_connection(tx.as_mut()).await?;
            tx.commit().await?;
            return Ok(PromptCacheConversationMaterializationRun {
                phase: PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE.to_string(),
                complete: true,
                page_complete: true,
                ..Default::default()
            });
        }
        update_prompt_cache_conversation_migration_progress_on_connection(
            tx.as_mut(),
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
            0,
            None,
        )
        .await?;
        tx.commit().await?;
        return Ok(PromptCacheConversationMaterializationRun {
            phase: PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL.to_string(),
            updated: 1,
            hit_scan_limit: true,
            page_complete: true,
            ..Default::default()
        });
    }

    run_prompt_cache_conversation_adaptive_key_batches(
        context,
        PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN,
        progress.source_max_invocation_id,
        false,
        &prompt_cache_keys,
    )
    .await
}

fn prompt_cache_conversation_never_yields() -> bool {
    false
}

async fn run_prompt_cache_conversations_materialization_with_policy(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
    policy: PromptCacheConversationBatchPolicy,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
    check_operator_enabled: bool,
) -> Result<PromptCacheConversationMaterializationRun> {
    let started_at = Instant::now();
    let mut progress = load_prompt_cache_conversation_migration_progress(pool).await?;
    let identity_repair_needed = progress.phase == PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE
        && prompt_cache_conversation_identity_repair_needed(pool).await?;
    if progress.phase == PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE
        && !identity_repair_needed
        && prompt_cache_conversation_materialization_is_complete(pool).await?
    {
        return Ok(PromptCacheConversationMaterializationRun {
            phase: PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE.to_string(),
            complete: true,
            page_complete: true,
            ..Default::default()
        });
    }
    if should_yield() {
        return Ok(PromptCacheConversationMaterializationRun {
            phase: progress.phase,
            hit_scan_limit: true,
            deferred: true,
            defer_reason: Some("coordinator_priority"),
            ..Default::default()
        });
    }
    mark_prompt_cache_conversation_stats_stale(pool).await?;

    if progress.phase == PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE {
        let identity_complete =
            prompt_cache_conversation_marker_exists(pool, PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME)
                .await?;
        let queue_pending = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT EXISTS(SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE})"
        ))
        .fetch_one(pool)
        .await?
            != 0;
        let phase = if !identity_complete || identity_repair_needed {
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL
        } else if queue_pending {
            PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN
        } else {
            PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD
        };
        update_prompt_cache_conversation_migration_progress(pool, phase, 0, None).await?;
        progress = load_prompt_cache_conversation_migration_progress(pool).await?;
    }

    let remaining_scan_limit = scan_limit as usize;
    if remaining_scan_limit == 0
        || prompt_cache_conversation_materialization_budget_exhausted(started_at, max_elapsed)
    {
        return Ok(PromptCacheConversationMaterializationRun {
            phase: progress.phase,
            hit_scan_limit: true,
            ..Default::default()
        });
    }
    let page_limit = PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE.min(remaining_scan_limit);
    let mut controller = PromptCacheConversationBatchController::for_policy(policy);
    let mut context = PromptCacheConversationMaterializationContext {
        pool,
        page_limit,
        started_at,
        max_elapsed,
        policy,
        controller: &mut controller,
        should_yield,
        check_operator_enabled,
    };
    let mut result = match progress.phase.as_str() {
        PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL => {
            run_prompt_cache_conversation_adaptive_identity_backfill_page(&mut context, &progress)
                .await?
        }
        PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION => {
            run_prompt_cache_conversation_adaptive_identity_reconciliation_page(
                &mut context,
                &progress,
            )
            .await?
        }
        PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD => {
            run_prompt_cache_conversation_adaptive_stats_rebuild_page(&mut context, &progress)
                .await?
        }
        PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN => {
            run_prompt_cache_conversation_adaptive_queue_drain_page(&mut context, &progress).await?
        }
        _ => {
            update_prompt_cache_conversation_migration_progress(
                pool,
                PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
                0,
                None,
            )
            .await?;
            PromptCacheConversationMaterializationRun {
                phase: PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL.to_string(),
                updated: 1,
                hit_scan_limit: true,
                page_complete: true,
                ..Default::default()
            }
        }
    };
    if prompt_cache_conversation_materialization_budget_exhausted(started_at, max_elapsed)
        && !result.complete
    {
        result.hit_scan_limit = true;
    }
    Ok(result)
}

pub(crate) async fn run_prompt_cache_conversations_materialization_with_pressure(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
) -> Result<PromptCacheConversationMaterializationRun> {
    run_prompt_cache_conversations_materialization_with_policy(
        pool,
        scan_limit,
        max_elapsed,
        PromptCacheConversationBatchPolicy::Adaptive,
        should_yield,
        false,
    )
    .await
}

pub(crate) async fn run_prompt_cache_conversations_materialization_with_pressure_and_control(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
) -> Result<PromptCacheConversationMaterializationRun> {
    run_prompt_cache_conversations_materialization_with_policy(
        pool,
        scan_limit,
        max_elapsed,
        PromptCacheConversationBatchPolicy::Adaptive,
        should_yield,
        true,
    )
    .await
}

pub(crate) async fn run_prompt_cache_conversations_materialization(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<PromptCacheConversationMaterializationRun> {
    run_prompt_cache_conversations_materialization_with_policy(
        pool,
        scan_limit,
        max_elapsed,
        PromptCacheConversationBatchPolicy::Adaptive,
        &prompt_cache_conversation_never_yields,
        false,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn run_prompt_cache_conversations_materialization_with_test_batch_size(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
    batch_size: usize,
) -> Result<PromptCacheConversationMaterializationRun> {
    run_prompt_cache_conversations_materialization_with_policy(
        pool,
        scan_limit,
        max_elapsed,
        PromptCacheConversationBatchPolicy::Fixed(batch_size),
        &prompt_cache_conversation_never_yields,
        false,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn run_prompt_cache_conversations_materialization_with_test_batch_size_and_pressure(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
    batch_size: usize,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
) -> Result<PromptCacheConversationMaterializationRun> {
    run_prompt_cache_conversations_materialization_with_policy(
        pool,
        scan_limit,
        max_elapsed,
        PromptCacheConversationBatchPolicy::Fixed(batch_size),
        should_yield,
        false,
    )
    .await
}

#[cfg(test)]
pub(crate) fn reset_prompt_cache_conversation_batch_controller_for_test() {
    *PROMPT_CACHE_CONVERSATION_BATCH_CONTROLLER
        .lock()
        .expect("prompt-cache batch controller mutex") =
        PromptCacheConversationBatchController::default();
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

async fn load_prompt_cache_conversation_row_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_key: &str,
) -> Result<Option<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT conversation_id, last_invoke_sequence \
         FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
    )
    .bind(prompt_cache_key)
    .fetch_optional(&mut *connection)
    .await
    .context("failed to load prompt-cache conversation identity")
}

async fn ensure_prompt_cache_conversation_row_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_key: &str,
) -> Result<bool> {
    if load_prompt_cache_conversation_row_on_connection(connection, prompt_cache_key)
        .await?
        .is_some()
    {
        return Ok(false);
    }
    let (_, created) =
        create_prompt_cache_conversation_row_on_connection(connection, prompt_cache_key).await?;
    Ok(created)
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

async fn conversation_id_exists_on_connection(
    connection: &mut SqliteConnection,
    conversation_id: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM prompt_cache_conversations WHERE conversation_id = ?1)",
    )
    .bind(conversation_id)
    .fetch_one(&mut *connection)
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

async fn conversation_prefix_conflicts_with_live_invocation_on_connection(
    connection: &mut SqliteConnection,
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
    .fetch_one(&mut *connection)
    .await?
        != 0)
}

async fn prompt_cache_conversation_id_candidate_conflicts_on_connection(
    connection: &mut SqliteConnection,
    conversation_id: &str,
) -> Result<bool> {
    let upper_bound = format!("{conversation_id}[");
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM prompt_cache_conversations WHERE conversation_id = ?1) \
         OR EXISTS(SELECT 1 FROM codex_invocations \
                   WHERE invoke_id >= ?1 AND invoke_id < ?2 AND length(invoke_id) = ?3)",
    )
    .bind(conversation_id)
    .bind(upper_bound)
    .bind(PROXY_INVOKE_ID_LENGTH as i64)
    .fetch_one(&mut *connection)
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

async fn create_prompt_cache_conversation_row_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_key: &str,
) -> Result<(PromptCacheConversationIdentity, bool)> {
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let conversation_id = generate_prompt_cache_conversation_id();
        if prompt_cache_conversation_id_candidate_conflicts_on_connection(
            connection,
            &conversation_id,
        )
        .await?
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
        .execute(&mut *connection)
        .await
        .with_context(|| {
            format!(
                "failed to persist prompt-cache conversation identity for key fingerprint {}",
                prompt_cache_key_fingerprint(prompt_cache_key)
            )
        })?;

        if insert_result.rows_affected() == 0 {
            if let Some((conversation_id, last_invoke_sequence)) =
                load_prompt_cache_conversation_row_on_connection(connection, prompt_cache_key)
                    .await?
            {
                return Ok((
                    PromptCacheConversationIdentity {
                        conversation_id,
                        next_sequence: last_invoke_sequence.max(0) as u32,
                    },
                    false,
                ));
            }
            continue;
        }

        info!(
            conversation_id = %conversation_id,
            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
            "prompt-cache conversation identity created"
        );
        return Ok((
            PromptCacheConversationIdentity {
                conversation_id,
                next_sequence: 0,
            },
            true,
        ));
    }

    bail!(
        "failed to allocate prompt-cache conversation id after {PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS} attempts"
    )
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
    if stats_rows.is_empty() {
        return Ok(0);
    }
    let mut update_query = sqlx::QueryBuilder::<Sqlite>::new(
        "WITH refreshed(prompt_cache_key, max_invoke_sequence, request_count, success_count, \
         failure_count, input_tokens, output_tokens, cache_input_tokens, \
         reported_cache_write_tokens, reasoning_tokens, total_tokens, cost, cost_input, \
         cost_cache_write, cost_cache_read, cost_output, cost_reasoning, first_invocation_at, \
         last_invocation_at) AS (VALUES ",
    );
    for (index, stats) in stats_rows.iter().enumerate() {
        if index > 0 {
            update_query.push(", ");
        }
        let max_sequence = stats.max_invoke_id.as_deref().and_then(|invoke_id| {
            let suffix = invoke_id_suffix(invoke_id, &stats.conversation_id)?;
            decode_prompt_cache_conversation_sequence(suffix)
        });
        update_query
            .push("(")
            .push_bind(&stats.prompt_cache_key)
            .push(", ")
            .push_bind(max_sequence.map(i64::from))
            .push(", ")
            .push_bind(stats.request_count)
            .push(", ")
            .push_bind(stats.success_count)
            .push(", ")
            .push_bind(stats.failure_count)
            .push(", ")
            .push_bind(stats.input_tokens)
            .push(", ")
            .push_bind(stats.output_tokens)
            .push(", ")
            .push_bind(stats.cache_input_tokens)
            .push(", ")
            .push_bind(stats.reported_cache_write_tokens)
            .push(", ")
            .push_bind(stats.reasoning_tokens)
            .push(", ")
            .push_bind(stats.total_tokens)
            .push(", ")
            .push_bind(stats.cost)
            .push(", ")
            .push_bind(stats.cost_input)
            .push(", ")
            .push_bind(stats.cost_cache_write)
            .push(", ")
            .push_bind(stats.cost_cache_read)
            .push(", ")
            .push_bind(stats.cost_output)
            .push(", ")
            .push_bind(stats.cost_reasoning)
            .push(", ")
            .push_bind(stats.first_invocation_at.as_deref())
            .push(", ")
            .push_bind(stats.last_invocation_at.as_deref())
            .push(")");
    }
    update_query.push(
        ") UPDATE prompt_cache_conversations AS c
         SET last_invoke_sequence = MAX(c.last_invoke_sequence, COALESCE(r.max_invoke_sequence, -1)),
             request_count = r.request_count,
             success_count = r.success_count,
             failure_count = r.failure_count,
             input_tokens = r.input_tokens,
             output_tokens = r.output_tokens,
             cache_input_tokens = r.cache_input_tokens,
             reported_cache_write_tokens = r.reported_cache_write_tokens,
             reasoning_tokens = r.reasoning_tokens,
             total_tokens = r.total_tokens,
             cost = r.cost,
             cost_input = r.cost_input,
             cost_cache_write = r.cost_cache_write,
             cost_cache_read = r.cost_cache_read,
             cost_output = r.cost_output,
             cost_reasoning = r.cost_reasoning,
             first_invocation_at = r.first_invocation_at,
             last_invocation_at = r.last_invocation_at,
             updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
         FROM refreshed AS r
         WHERE c.prompt_cache_key = r.prompt_cache_key",
    );
    update_query.build().execute(&mut *connection).await?;
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
    let materialization_complete = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(\
            SELECT 1 \
            FROM {PROMPT_CACHE_CONVERSATIONS_MIGRATION_PROGRESS_TABLE} AS progress \
            WHERE progress.migration_name = ?1 \
              AND progress.phase = ?2 \
              AND EXISTS (\
                  SELECT 1 FROM schema_refresh_migrations \
                  WHERE migration_name = ?3\
              ) \
              AND NOT EXISTS (\
                  SELECT 1 FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE}\
              )\
        )"
    ))
    .bind(PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION_NAME)
    .bind(PROMPT_CACHE_CONVERSATIONS_PHASE_COMPLETE)
    .bind(PROMPT_CACHE_CONVERSATIONS_BACKFILL_NAME)
    .fetch_one(&mut *connection)
    .await?
        != 0;
    if materialization_complete {
        sqlx::query(
            "INSERT OR REPLACE INTO schema_refresh_migrations (migration_name) VALUES (?1)",
        )
        .bind(PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME)
        .execute(&mut *connection)
        .await
        .context("failed to mark prompt-cache conversation statistics fresh")?;
    } else {
        mark_prompt_cache_conversation_stats_stale_on_connection(connection).await?;
    }
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

    #[test]
    fn adaptive_batch_controller_respects_latency_and_pressure_bounds() {
        let mut controller = PromptCacheConversationBatchController::default();
        assert_eq!(controller.batch_size, 64);

        controller.observe_success(50, false);
        assert_eq!(controller.batch_size, 64);
        controller.observe_success(50, false);
        assert_eq!(controller.batch_size, 128);

        controller.observe_success(49, false);
        controller.observe_success(49, false);
        assert_eq!(controller.batch_size, 256);
        controller.observe_success(49, false);
        controller.observe_success(49, false);
        assert_eq!(controller.batch_size, 400);

        controller.observe_success(200, false);
        assert_eq!(controller.batch_size, 200);
        controller.observe_success(1, true);
        assert_eq!(controller.batch_size, 100);
        controller.observe_failure();
        assert_eq!(controller.batch_size, 50);
        controller.observe_failure();
        assert_eq!(controller.batch_size, 32);
        controller.observe_failure();
        assert_eq!(controller.batch_size, 32);
    }

    #[test]
    fn adaptive_batch_controller_never_exceeds_logical_page_or_minimum() {
        let mut controller = PromptCacheConversationBatchController::default();
        for _ in 0..20 {
            controller.observe_success(1, false);
            controller.observe_success(1, false);
        }
        assert_eq!(
            controller.batch_size,
            PROMPT_CACHE_CONVERSATION_BATCH_MAX_SIZE
        );
        assert_eq!(controller.next_batch_size(7), 7);

        controller.observe_success(200, false);
        for _ in 0..10 {
            controller.observe_failure();
        }
        assert_eq!(
            controller.batch_size,
            PROMPT_CACHE_CONVERSATION_BATCH_MIN_SIZE
        );
        assert_eq!(controller.next_batch_size(7), 7);
    }
}
