use super::*;

const PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET: Duration = Duration::from_secs(2);
const PROMPT_CACHE_CONVERSATION_STATS_PROGRESS_OPS: i32 = 1_000;
const PROMPT_CACHE_ORPHAN_CLEANUP_BUDGET_EXPIRED: &str =
    "prompt-cache orphan cleanup exceeded the retention work budget";

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
const PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE: &str =
    "prompt_cache_conversation_stats_generation_clock";
const PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE: &str =
    "prompt_cache_conversation_stats_refresh_staging";
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
const PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_PAGE_SIZE: usize = 32;
const PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE: &str =
    "prompt_cache_conversation_orphan_cleanup_state";
const PROMPT_CACHE_CONVERSATION_STATS_PAGE_INITIAL_SIZE: i64 = 256;

struct PromptCacheConversationStatsProgressConnection {
    connection: sqlx::pool::PoolConnection<Sqlite>,
    progress_handler_installed: bool,
}

impl PromptCacheConversationStatsProgressConnection {
    fn close_on_drop(&mut self) {
        self.connection.close_on_drop();
    }
}

impl Drop for PromptCacheConversationStatsProgressConnection {
    fn drop(&mut self) {
        if self.progress_handler_installed {
            self.connection.close_on_drop();
        }
    }
}

static PROMPT_CACHE_CONVERSATION_BATCH_CONTROLLER: Lazy<
    std::sync::Mutex<PromptCacheConversationBatchController>,
> = Lazy::new(|| std::sync::Mutex::new(PromptCacheConversationBatchController::default()));

/// Prefixes assigned to unbound invocations are process-local state. Keep the
/// namespace separate from the hot allocation locks so only a new hourly
/// prefix initialization contends on it.
static PROMPT_CACHE_UNBOUND_PREFIX_NAMESPACE: Lazy<Mutex<HashSet<String>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationIdentity {
    pub(crate) conversation_id: String,
    pub(crate) next_sequence: u32,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PromptCacheConversationIdentityCache {
    pub(crate) conversations: HashMap<String, PromptCacheConversationIdentity>,
    pub(crate) active_prompt_cache_keys: HashMap<String, usize>,
    /// Per-key allocation locks keep slow identity/sequence SQL from blocking
    /// unrelated prompt-cache conversations. Weak entries are reclaimed when no
    /// allocator still owns the corresponding lock.
    pub(crate) allocation_locks: HashMap<String, std::sync::Weak<Mutex<()>>>,
    /// Unbound IDs share one lock because their hourly prefix is process-local.
    pub(crate) unbound_allocation_lock: Arc<Mutex<()>>,
    pub(crate) unbound_prefix: Option<UnboundInvokePrefix>,
}

#[derive(Debug, Clone)]
pub(crate) struct UnboundInvokePrefix {
    pub(crate) hour_key: i64,
    pub(crate) prefix: String,
    pub(crate) next_sequence: u32,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
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

#[derive(Debug, FromRow)]
struct PromptCacheConversationInvocationRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    status: Option<String>,
    error_message: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reported_cache_write_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    cost_input: Option<f64>,
    cost_cache_write: Option<f64>,
    cost_cache_read: Option<f64>,
    cost_output: Option<f64>,
    cost_reasoning: Option<f64>,
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
    pub(crate) control_generation_changed: bool,
    pub(crate) batch_count: u64,
    pub(crate) last_batch_size: usize,
    pub(crate) max_batch_size: usize,
    pub(crate) batch_elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptCacheMaterializationControlStop {
    Disabled,
    GenerationChanged,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptCacheStatsPageOutcome {
    Complete,
    Pending,
    GenerationChanged,
    BudgetExhausted,
    Disabled,
    ControlGenerationChanged,
    Unavailable,
}

struct PromptCacheMaterializationBatchWork {
    identities_created: usize,
    refreshed: usize,
    scanned: usize,
    elapsed: Duration,
}

enum PromptCacheMaterializationBatchOutcome {
    Complete(PromptCacheMaterializationBatchWork),
    Deferred {
        work: PromptCacheMaterializationBatchWork,
        reason: Option<&'static str>,
        control_generation_changed: bool,
    },
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
    control: Option<&'a Arc<crate::maintenance_store::PromptCacheMaterializationControl>>,
    control_generation: Option<u64>,
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
            generation INTEGER NOT NULL DEFAULT 1,\
            enqueued_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache conversation statistics refresh queue")?;
    ensure_prompt_cache_refresh_queue_column(pool, "generation", "INTEGER NOT NULL DEFAULT 1")
        .await?;
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} (\
            prompt_cache_key TEXT PRIMARY KEY,\
            generation INTEGER NOT NULL DEFAULT 0\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache conversation statistics generation clock")?;
    sqlx::query(&format!(
        "INSERT OR IGNORE INTO {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} (prompt_cache_key,generation) SELECT prompt_cache_key,generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE}"
    ))
    .execute(pool)
    .await
    .context("failed to seed prompt-cache statistics generation clock")?;
    sqlx::query(&format!(
        "UPDATE {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} AS clock SET generation=MAX(clock.generation,COALESCE((SELECT queue.generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} AS queue WHERE queue.prompt_cache_key=clock.prompt_cache_key),clock.generation))"
    ))
    .execute(pool)
    .await
    .context("failed to repair prompt-cache statistics generation clock")?;
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} (\
            prompt_cache_key TEXT PRIMARY KEY,\
            generation INTEGER NOT NULL,\
            source_max_invocation_id INTEGER NOT NULL,\
            cursor_occurred_at TEXT,\
            cursor_id INTEGER NOT NULL DEFAULT 0,\
            accumulator_json TEXT NOT NULL,\
            page_size INTEGER NOT NULL DEFAULT {PROMPT_CACHE_CONVERSATION_STATS_PAGE_INITIAL_SIZE},\
            updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache conversation statistics staging")?;
    for (column, definition) in [
        ("cursor_occurred_at", "TEXT"),
        ("cursor_id", "INTEGER NOT NULL DEFAULT 0"),
        ("page_size", "INTEGER NOT NULL DEFAULT 256"),
    ] {
        ensure_prompt_cache_refresh_staging_column(pool, column, definition).await?;
    }
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE} (\
            scope TEXT PRIMARY KEY,\
            epoch INTEGER NOT NULL DEFAULT 0,\
            cursor_key TEXT,\
            updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now'))\
        )"
    ))
    .execute(pool)
    .await
    .context("failed to ensure prompt-cache orphan cleanup state")?;
    sqlx::query(&format!(
        "INSERT OR IGNORE INTO {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE} (scope) VALUES ('prompt_cache_conversations')"
    ))
    .execute(pool)
    .await?;

    let old_prompt_cache_key_expr = invocation_prompt_cache_key_expr_sql("OLD");
    let new_prompt_cache_key_expr = invocation_prompt_cache_key_expr_sql("NEW");
    let refresh_queue_table = PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE;
    let stats_marker = PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_NAME;
    let mut trigger_tx = pool
        .begin()
        .await
        .context("failed to begin prompt-cache trigger replacement")?;
    for (trigger_name, trigger_sql) in [
        (
            "prompt_cache_conversations_stats_enqueue_insert",
            format!(
                "CREATE TRIGGER prompt_cache_conversations_stats_enqueue_insert \
                 AFTER INSERT ON codex_invocations \
                 WHEN {new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> '' \
                 BEGIN \
                   INSERT INTO {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} (prompt_cache_key,generation) VALUES ({new_prompt_cache_key_expr},1) \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = generation + 1; \
                   INSERT INTO {refresh_queue_table} (prompt_cache_key,generation) SELECT prompt_cache_key,generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} WHERE prompt_cache_key = {new_prompt_cache_key_expr} \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = excluded.generation, enqueued_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'); \
                   DELETE FROM schema_refresh_migrations WHERE migration_name = '{stats_marker}'; \
                 END"
            ),
        ),
        (
            "prompt_cache_conversations_stats_enqueue_update",
            format!(
                "CREATE TRIGGER prompt_cache_conversations_stats_enqueue_update \
                 AFTER UPDATE OF payload, status, error_message, input_tokens, output_tokens, cache_input_tokens, reported_cache_write_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, occurred_at, invoke_id ON codex_invocations \
                 WHEN ({old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> '') \
                   OR ({new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> '') \
                 BEGIN \
                   INSERT INTO {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} (prompt_cache_key,generation) SELECT {old_prompt_cache_key_expr},1 WHERE {old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> '' \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = generation + 1; \
                   INSERT INTO {refresh_queue_table} (prompt_cache_key,generation) SELECT prompt_cache_key,generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} WHERE prompt_cache_key = {old_prompt_cache_key_expr} AND {old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> '' \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = excluded.generation, enqueued_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'); \
                   INSERT INTO {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} (prompt_cache_key,generation) SELECT {new_prompt_cache_key_expr},1 WHERE {new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> '' AND ({old_prompt_cache_key_expr} IS NULL OR {new_prompt_cache_key_expr} <> {old_prompt_cache_key_expr}) \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = generation + 1; \
                   INSERT INTO {refresh_queue_table} (prompt_cache_key,generation) SELECT prompt_cache_key,generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} WHERE prompt_cache_key = {new_prompt_cache_key_expr} AND {new_prompt_cache_key_expr} IS NOT NULL AND {new_prompt_cache_key_expr} <> '' AND ({old_prompt_cache_key_expr} IS NULL OR {new_prompt_cache_key_expr} <> {old_prompt_cache_key_expr}) \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = excluded.generation, enqueued_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'); \
                   DELETE FROM schema_refresh_migrations WHERE migration_name = '{stats_marker}'; \
                 END"
            ),
        ),
        (
            "prompt_cache_conversations_stats_enqueue_delete",
            format!(
                "CREATE TRIGGER prompt_cache_conversations_stats_enqueue_delete \
                 AFTER DELETE ON codex_invocations \
                 WHEN {old_prompt_cache_key_expr} IS NOT NULL AND {old_prompt_cache_key_expr} <> '' \
                 BEGIN \
                   INSERT INTO {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} (prompt_cache_key,generation) VALUES ({old_prompt_cache_key_expr},1) \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = generation + 1; \
                   INSERT INTO {refresh_queue_table} (prompt_cache_key,generation) SELECT prompt_cache_key,generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} WHERE prompt_cache_key = {old_prompt_cache_key_expr} \
                     ON CONFLICT(prompt_cache_key) DO UPDATE SET generation = excluded.generation, enqueued_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'); \
                   DELETE FROM schema_refresh_migrations WHERE migration_name = '{stats_marker}'; \
                 END"
            ),
        ),
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(trigger_tx.as_mut())
            .await
            .with_context(|| format!("failed to replace trigger {trigger_name}"))?;
        sqlx::query(&trigger_sql)
            .execute(trigger_tx.as_mut())
            .await
            .with_context(|| format!("failed to ensure trigger {trigger_name}"))?;
    }
    trigger_tx
        .commit()
        .await
        .context("failed to commit prompt-cache trigger replacement")?;

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

async fn ensure_prompt_cache_refresh_queue_column(
    pool: &Pool<Sqlite>,
    column_name: &str,
    definition: &str,
) -> Result<()> {
    let columns = sqlx::query("PRAGMA table_info(prompt_cache_conversation_stats_refresh_queue)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    if columns.contains(column_name) {
        return Ok(());
    }
    sqlx::query(&format!(
        "ALTER TABLE {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} ADD COLUMN {column_name} {definition}"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_prompt_cache_refresh_staging_column(
    pool: &Pool<Sqlite>,
    column_name: &str,
    definition: &str,
) -> Result<()> {
    let columns = sqlx::query("PRAGMA table_info(prompt_cache_conversation_stats_refresh_staging)")
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    if columns.contains(column_name) {
        return Ok(());
    }
    sqlx::query(&format!(
        "ALTER TABLE {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} ADD COLUMN {column_name} {definition}"
    ))
    .execute(pool)
    .await?;
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
    maintenance_pool: &Pool<Sqlite>,
    control: &crate::maintenance_store::PromptCacheMaterializationControl,
) -> Result<PromptCacheConversationMaterializationStatus> {
    let progress = load_prompt_cache_conversation_migration_progress(pool).await?;
    let control = control
        .snapshot()
        .ok_or_else(|| anyhow!("maintenance database control is unavailable"))?;
    let task = crate::load_startup_backfill_progress_from_pool(
        maintenance_pool,
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
        enabled: control.enabled,
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

async fn update_prompt_cache_conversation_migration_progress_with_control(
    pool: &Pool<Sqlite>,
    phase: &str,
    source_max_invocation_id: i64,
    cursor_key: Option<&str>,
    control: Option<&Arc<crate::maintenance_store::PromptCacheMaterializationControl>>,
    expected_generation: Option<u64>,
) -> Result<std::result::Result<(), PromptCacheMaterializationControlStop>> {
    let step = match prompt_cache_conversation_begin_control_step(control, expected_generation) {
        Ok(step) => step,
        Err(stop) => return Ok(Err(stop)),
    };
    let mut tx = pool.begin().await?;
    update_prompt_cache_conversation_migration_progress_on_connection(
        tx.as_mut(),
        phase,
        source_max_invocation_id,
        cursor_key,
    )
    .await?;
    tx.commit().await?;
    drop(step);
    Ok(Ok(()))
}

fn prompt_cache_conversation_materialization_deferred(
    phase: &str,
    reason: Option<&'static str>,
) -> PromptCacheConversationMaterializationRun {
    PromptCacheConversationMaterializationRun {
        phase: phase.to_string(),
        hit_scan_limit: true,
        deferred: true,
        defer_reason: reason,
        control_generation_changed: reason.is_none(),
        ..Default::default()
    }
}

fn prompt_cache_materialization_defer_reason(
    stop: PromptCacheMaterializationControlStop,
) -> Option<&'static str> {
    match stop {
        PromptCacheMaterializationControlStop::Disabled => Some("operator_disabled"),
        PromptCacheMaterializationControlStop::GenerationChanged => None,
        PromptCacheMaterializationControlStop::Unavailable => {
            Some("maintenance_database_unavailable")
        }
    }
}

fn prompt_cache_conversation_begin_control_step(
    control: Option<&std::sync::Arc<crate::maintenance_store::PromptCacheMaterializationControl>>,
    expected_generation: Option<u64>,
) -> std::result::Result<
    Option<crate::maintenance_store::PromptCacheMaterializationStep>,
    PromptCacheMaterializationControlStop,
> {
    let Some(control) = control else {
        return Ok(None);
    };
    match control.begin_step(expected_generation) {
        crate::maintenance_store::PromptCacheMaterializationStepAdmission::Started(step) => {
            Ok(Some(step))
        }
        crate::maintenance_store::PromptCacheMaterializationStepAdmission::Disabled => {
            Err(PromptCacheMaterializationControlStop::Disabled)
        }
        crate::maintenance_store::PromptCacheMaterializationStepAdmission::GenerationChanged => {
            Err(PromptCacheMaterializationControlStop::GenerationChanged)
        }
        crate::maintenance_store::PromptCacheMaterializationStepAdmission::Unavailable => {
            Err(PromptCacheMaterializationControlStop::Unavailable)
        }
    }
}

fn prompt_cache_materialization_control_deferred(
    phase: &str,
    stop: PromptCacheMaterializationControlStop,
) -> PromptCacheConversationMaterializationRun {
    prompt_cache_conversation_materialization_deferred(
        phase,
        prompt_cache_materialization_defer_reason(stop),
    )
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
    control: Option<&std::sync::Arc<crate::maintenance_store::PromptCacheMaterializationControl>>,
    expected_generation: Option<u64>,
    prompt_cache_keys: &[String],
) -> Result<PromptCacheMaterializationBatchOutcome> {
    let started_at = Instant::now();
    let mut identities_created = 0;
    let identity_step =
        match prompt_cache_conversation_begin_control_step(control, expected_generation) {
            Ok(step) => step,
            Err(stop) => {
                return Ok(PromptCacheMaterializationBatchOutcome::Deferred {
                    work: PromptCacheMaterializationBatchWork {
                        identities_created,
                        refreshed: 0,
                        scanned: 0,
                        elapsed: started_at.elapsed(),
                    },
                    reason: prompt_cache_materialization_defer_reason(stop),
                    control_generation_changed: stop
                        == PromptCacheMaterializationControlStop::GenerationChanged,
                });
            }
        };
    {
        let mut tx = pool.begin().await?;
        let existing_keys =
            load_prompt_cache_conversation_keys_on_connection(tx.as_mut(), prompt_cache_keys)
                .await?;
        for prompt_cache_key in prompt_cache_keys {
            if !existing_keys.contains(prompt_cache_key) {
                let (_, created) = create_prompt_cache_conversation_row_on_connection(
                    tx.as_mut(),
                    prompt_cache_key,
                )
                .await?;
                if created {
                    identities_created += 1;
                }
            }
        }
        tx.commit().await?;
    }
    drop(identity_step);

    let mut refreshed = 0;
    for prompt_cache_key in prompt_cache_keys {
        let stats_outcome = match refresh_prompt_cache_conversation_stats_bounded_page(
            pool,
            prompt_cache_key,
            control,
            expected_generation,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) if prompt_cache_statistics_budget_error(&error) => {
                PromptCacheStatsPageOutcome::BudgetExhausted
            }
            Err(error) => return Err(error),
        };
        let (reason, control_generation_changed) = match stats_outcome {
            PromptCacheStatsPageOutcome::Complete => {
                refreshed += 1;
                continue;
            }
            PromptCacheStatsPageOutcome::Pending => (Some("stats_page_pending"), false),
            PromptCacheStatsPageOutcome::GenerationChanged => {
                (Some("stats_generation_changed"), false)
            }
            PromptCacheStatsPageOutcome::BudgetExhausted => (Some("stats_budget_exhausted"), false),
            PromptCacheStatsPageOutcome::Disabled => (Some("operator_disabled"), false),
            PromptCacheStatsPageOutcome::ControlGenerationChanged => (None, true),
            PromptCacheStatsPageOutcome::Unavailable => {
                (Some("maintenance_database_unavailable"), false)
            }
        };
        return Ok(PromptCacheMaterializationBatchOutcome::Deferred {
            work: PromptCacheMaterializationBatchWork {
                identities_created,
                refreshed,
                scanned: prompt_cache_keys.len(),
                elapsed: started_at.elapsed(),
            },
            reason,
            control_generation_changed,
        });
    }

    let cursor_step =
        match prompt_cache_conversation_begin_control_step(control, expected_generation) {
            Ok(step) => step,
            Err(stop) => {
                return Ok(PromptCacheMaterializationBatchOutcome::Deferred {
                    work: PromptCacheMaterializationBatchWork {
                        identities_created,
                        refreshed,
                        scanned: prompt_cache_keys.len(),
                        elapsed: started_at.elapsed(),
                    },
                    reason: prompt_cache_materialization_defer_reason(stop),
                    control_generation_changed: stop
                        == PromptCacheMaterializationControlStop::GenerationChanged,
                });
            }
        };
    let mut tx = pool.begin().await?;
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
    drop(cursor_step);
    Ok(PromptCacheMaterializationBatchOutcome::Complete(
        PromptCacheMaterializationBatchWork {
            identities_created,
            refreshed,
            scanned: prompt_cache_keys.len(),
            elapsed: started_at.elapsed(),
        },
    ))
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
            result.deferred = true;
            result.defer_reason = Some("stats_budget_exhausted");
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
        let batch_outcome = match prompt_cache_conversation_materialize_key_batch(
            context.pool,
            phase,
            source_max_invocation_id,
            advance_cursor,
            context.control,
            context.control_generation,
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
        let (work, complete) = match batch_outcome {
            PromptCacheMaterializationBatchOutcome::Complete(work) => (work, true),
            PromptCacheMaterializationBatchOutcome::Deferred {
                work,
                reason,
                control_generation_changed,
            } => {
                if work.scanned > 0 {
                    result.batch_count = result.batch_count.saturating_add(1);
                    result.last_batch_size = work.scanned;
                    result.max_batch_size = result.max_batch_size.max(work.scanned);
                    result.batch_elapsed_ms = result
                        .batch_elapsed_ms
                        .saturating_add(work.elapsed.as_millis().min(u128::from(u64::MAX)) as u64);
                    result.scanned = result.scanned.saturating_add(work.scanned as u64);
                    result.updated = result.updated.saturating_add(
                        work.identities_created.saturating_add(work.refreshed) as u64,
                    );
                }
                result.deferred = true;
                result.defer_reason = reason;
                result.control_generation_changed = control_generation_changed;
                context.controller.persist_adaptive(context.policy);
                return Ok(result);
            }
        };
        let priority_waiter = (context.should_yield)();
        if complete {
            context
                .controller
                .observe_success(work.elapsed.as_millis(), priority_waiter);
            context.controller.persist_adaptive(context.policy);
        }

        offset = batch_end;
        result.batch_count = result.batch_count.saturating_add(1);
        result.last_batch_size = work.scanned;
        result.max_batch_size = result.max_batch_size.max(work.scanned);
        result.batch_elapsed_ms = result
            .batch_elapsed_ms
            .saturating_add(work.elapsed.as_millis().min(u128::from(u64::MAX)) as u64);
        result.scanned = result.scanned.saturating_add(work.scanned as u64);
        result.updated = result
            .updated
            .saturating_add(work.identities_created.saturating_add(work.refreshed) as u64);

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
        if let Err(stop) = update_prompt_cache_conversation_migration_progress_with_control(
            context.pool,
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
            source_max_invocation_id,
            progress.cursor_key.as_deref(),
            context.control,
            context.control_generation,
        )
        .await?
        {
            return Ok(prompt_cache_materialization_control_deferred(
                PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
                stop,
            ));
        }
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
        if let Err(stop) = update_prompt_cache_conversation_migration_progress_with_control(
            context.pool,
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION,
            source_max_invocation_id,
            None,
            context.control,
            context.control_generation,
        )
        .await?
        {
            return Ok(prompt_cache_materialization_control_deferred(
                PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
                stop,
            ));
        }
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
    if let Err(stop) = update_prompt_cache_conversation_migration_progress_with_control(
        context.pool,
        next_phase,
        source_max_invocation_id,
        next_cursor,
        context.control,
        context.control_generation,
    )
    .await?
    {
        return Ok(prompt_cache_materialization_control_deferred(
            PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
            stop,
        ));
    }
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
        let _step = match prompt_cache_conversation_begin_control_step(
            context.control,
            context.control_generation,
        ) {
            Ok(step) => step,
            Err(stop) => {
                return Ok(prompt_cache_materialization_control_deferred(
                    PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_RECONCILIATION,
                    stop,
                ));
            }
        };
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
        let _step = match prompt_cache_conversation_begin_control_step(
            context.control,
            context.control_generation,
        ) {
            Ok(step) => step,
            Err(stop) => {
                return Ok(prompt_cache_materialization_control_deferred(
                    PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD,
                    stop,
                ));
            }
        };
        let mut tx = context.pool.begin().await?;
        update_prompt_cache_conversation_migration_progress_on_connection(
            tx.as_mut(),
            PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN,
            progress.source_max_invocation_id,
            None,
        )
        .await?;
        tx.commit().await?;
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
    if let Err(stop) = update_prompt_cache_conversation_migration_progress_with_control(
        context.pool,
        next_phase,
        progress.source_max_invocation_id,
        next_cursor,
        context.control,
        context.control_generation,
    )
    .await?
    {
        return Ok(prompt_cache_materialization_control_deferred(
            PROMPT_CACHE_CONVERSATIONS_PHASE_STATS_REBUILD,
            stop,
        ));
    }
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
        let _step = match prompt_cache_conversation_begin_control_step(
            context.control,
            context.control_generation,
        ) {
            Ok(step) => step,
            Err(stop) => {
                return Ok(prompt_cache_materialization_control_deferred(
                    PROMPT_CACHE_CONVERSATIONS_PHASE_QUEUE_DRAIN,
                    stop,
                ));
            }
        };
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
    control: Option<&std::sync::Arc<crate::maintenance_store::PromptCacheMaterializationControl>>,
    expected_generation: Option<u64>,
) -> Result<PromptCacheConversationMaterializationRun> {
    let started_at = Instant::now();
    let run_step = match prompt_cache_conversation_begin_control_step(control, expected_generation)
    {
        Ok(step) => step,
        Err(stop) => {
            return Ok(prompt_cache_materialization_control_deferred(
                PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
                stop,
            ));
        }
    };
    let control_generation = run_step.as_ref().map(|step| step.generation);
    drop(run_step);
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
    let stale_step = match prompt_cache_conversation_begin_control_step(control, control_generation)
    {
        Ok(step) => step,
        Err(stop) => {
            return Ok(prompt_cache_materialization_control_deferred(
                &progress.phase,
                stop,
            ));
        }
    };
    mark_prompt_cache_conversation_stats_stale(pool).await?;
    drop(stale_step);

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
        if let Err(stop) = update_prompt_cache_conversation_migration_progress_with_control(
            pool,
            phase,
            0,
            None,
            control,
            control_generation,
        )
        .await?
        {
            return Ok(prompt_cache_materialization_control_deferred(
                &progress.phase,
                stop,
            ));
        }
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
        control,
        control_generation,
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
            if let Err(stop) = update_prompt_cache_conversation_migration_progress_with_control(
                pool,
                PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
                0,
                None,
                control,
                control_generation,
            )
            .await?
            {
                return Ok(prompt_cache_materialization_control_deferred(
                    PROMPT_CACHE_CONVERSATIONS_PHASE_IDENTITY_BACKFILL,
                    stop,
                ));
            }
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
        None,
        None,
    )
    .await
}

pub(crate) async fn run_prompt_cache_conversations_materialization_with_pressure_and_control(
    pool: &Pool<Sqlite>,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
    should_yield: &(dyn Fn() -> bool + Send + Sync),
    control: &std::sync::Arc<crate::maintenance_store::PromptCacheMaterializationControl>,
    expected_generation: u64,
) -> Result<PromptCacheConversationMaterializationRun> {
    run_prompt_cache_conversations_materialization_with_policy(
        pool,
        scan_limit,
        max_elapsed,
        PromptCacheConversationBatchPolicy::Adaptive,
        should_yield,
        Some(control),
        Some(expected_generation),
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
        None,
        None,
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
        None,
        None,
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
        None,
        None,
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

async fn prompt_cache_conversation_id_candidate_conflicts(
    pool: &Pool<Sqlite>,
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
    .fetch_one(pool)
    .await?
        != 0)
}

async fn create_prompt_cache_conversation_row(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
) -> Result<PromptCacheConversationIdentity> {
    let namespace = PROMPT_CACHE_UNBOUND_PREFIX_NAMESPACE.lock().await;
    create_prompt_cache_conversation_row_with_generator_and_exclusions(
        pool,
        prompt_cache_key,
        generate_prompt_cache_conversation_id,
        Some(&namespace),
    )
    .await
}

async fn create_prompt_cache_conversation_row_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_key: &str,
) -> Result<(PromptCacheConversationIdentity, bool)> {
    let namespace = PROMPT_CACHE_UNBOUND_PREFIX_NAMESPACE.lock().await;
    create_prompt_cache_conversation_row_on_connection_with_exclusions(
        connection,
        prompt_cache_key,
        &namespace,
    )
    .await
}

async fn create_prompt_cache_conversation_row_on_connection_with_exclusions(
    connection: &mut SqliteConnection,
    prompt_cache_key: &str,
    excluded_prefixes: &HashSet<String>,
) -> Result<(PromptCacheConversationIdentity, bool)> {
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let conversation_id = generate_prompt_cache_conversation_id();
        if excluded_prefixes.contains(&conversation_id)
            || prompt_cache_conversation_id_candidate_conflicts_on_connection(
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
    create_prompt_cache_conversation_row_with_generator_and_exclusions(
        pool,
        prompt_cache_key,
        &mut generate,
        None,
    )
    .await
}

async fn create_prompt_cache_conversation_row_with_generator_and_exclusions<F>(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    mut generate: F,
    excluded_prefixes: Option<&HashSet<String>>,
) -> Result<PromptCacheConversationIdentity>
where
    F: FnMut() -> String,
{
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let conversation_id = generate();
        if excluded_prefixes.is_some_and(|prefixes| prefixes.contains(&conversation_id))
            || conversation_id_exists(pool, &conversation_id).await?
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

fn allocation_lock_for_cache(
    cache: &mut PromptCacheConversationIdentityCache,
    prompt_cache_key: Option<&str>,
) -> Arc<Mutex<()>> {
    cache
        .allocation_locks
        .retain(|_, lock| lock.strong_count() > 0);
    if let Some(prompt_cache_key) = prompt_cache_key {
        if let Some(lock) = cache
            .allocation_locks
            .get(prompt_cache_key)
            .and_then(std::sync::Weak::upgrade)
        {
            return lock;
        }
        let lock = Arc::new(Mutex::new(()));
        cache
            .allocation_locks
            .insert(prompt_cache_key.to_string(), Arc::downgrade(&lock));
        lock
    } else {
        cache.unbound_allocation_lock.clone()
    }
}

async fn prompt_cache_allocation_lock(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Arc<Mutex<()>> {
    let mut cache = state.prompt_cache_conversation_cache.lock().await;
    allocation_lock_for_cache(&mut cache.identity_cache, prompt_cache_key)
}

#[derive(Debug)]
enum PromptCacheInvokeIdCacheUpdate {
    Conversation {
        conversation_id: String,
        next_sequence: u32,
    },
    Unbound {
        prefix: UnboundInvokePrefix,
    },
}

#[derive(Debug)]
struct PromptCacheInvokeIdAllocation {
    invoke_id: String,
    cache_update: PromptCacheInvokeIdCacheUpdate,
}

async fn apply_prompt_cache_invoke_id_cache_update(
    state: &AppState,
    prompt_cache_key: Option<&str>,
    cache_update: PromptCacheInvokeIdCacheUpdate,
) {
    let mut cache = state.prompt_cache_conversation_cache.lock().await;
    match cache_update {
        PromptCacheInvokeIdCacheUpdate::Conversation {
            conversation_id,
            next_sequence,
        } => {
            if let Some(prompt_cache_key) = prompt_cache_key
                && let Some(identity) = cache.identity_cache.conversations.get_mut(prompt_cache_key)
            {
                identity.conversation_id = conversation_id;
                identity.next_sequence = next_sequence;
            }
        }
        PromptCacheInvokeIdCacheUpdate::Unbound { prefix } => {
            cache.identity_cache.unbound_prefix = Some(prefix);
        }
    }
}

async fn initialize_unbound_prompt_cache_prefix(
    pool: &Pool<Sqlite>,
    hour_key: i64,
) -> Result<UnboundInvokePrefix> {
    let mut namespace = PROMPT_CACHE_UNBOUND_PREFIX_NAMESPACE.lock().await;
    for _ in 0..PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS {
        let candidate = generate_prompt_cache_conversation_id();
        if namespace.contains(&candidate)
            || prompt_cache_conversation_id_candidate_conflicts(pool, &candidate).await?
        {
            continue;
        }
        namespace.insert(candidate.clone());
        return Ok(UnboundInvokePrefix {
            hour_key,
            prefix: candidate,
            next_sequence: 0,
        });
    }
    bail!(
        "failed to allocate unbound prompt-cache invoke prefix after {PROMPT_CACHE_CONVERSATION_ID_GENERATION_ATTEMPTS} attempts"
    )
}

pub(crate) async fn allocate_proxy_invoke_id(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Result<String> {
    let prompt_cache_key = normalize_prompt_cache_key(prompt_cache_key).map(ToOwned::to_owned);
    let allocation_lock = prompt_cache_allocation_lock(state, prompt_cache_key.as_deref()).await;
    let _allocation_guard = allocation_lock.lock().await;
    let allocation =
        allocate_proxy_invoke_id_serialized(state, prompt_cache_key.as_deref()).await?;
    let invoke_id = allocation.invoke_id.clone();
    apply_prompt_cache_invoke_id_cache_update(
        state,
        prompt_cache_key.as_deref(),
        allocation.cache_update,
    )
    .await;
    Ok(invoke_id)
}

pub(crate) async fn allocate_proxy_invoke_id_with_active_lease(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Result<String> {
    let prompt_cache_key = normalize_prompt_cache_key(prompt_cache_key).map(ToOwned::to_owned);
    let allocation_lock = {
        let mut cache = state.prompt_cache_conversation_cache.lock().await;
        let allocation_lock =
            allocation_lock_for_cache(&mut cache.identity_cache, prompt_cache_key.as_deref());
        if let Some(prompt_cache_key) = prompt_cache_key.as_deref() {
            let leases = cache
                .identity_cache
                .active_prompt_cache_keys
                .entry(prompt_cache_key.to_string())
                .or_default();
            *leases = leases.saturating_add(1);
        }
        allocation_lock
    };

    let lease_guard = prompt_cache_key.as_deref().map(|prompt_cache_key| {
        PromptCacheConversationLeaseDropGuard::new(
            state.prompt_cache_conversation_cache.clone(),
            Some(prompt_cache_key),
        )
    });
    let _allocation_guard = allocation_lock.lock().await;
    let result = allocate_proxy_invoke_id_serialized(state, prompt_cache_key.as_deref()).await;
    match result {
        Ok(allocation) => {
            let invoke_id = allocation.invoke_id.clone();
            apply_prompt_cache_invoke_id_cache_update(
                state,
                prompt_cache_key.as_deref(),
                allocation.cache_update,
            )
            .await;
            if let Some(mut lease_guard) = lease_guard {
                lease_guard.disarm();
            }
            Ok(invoke_id)
        }
        Err(err) => {
            if let Some(prompt_cache_key) = prompt_cache_key.as_deref() {
                release_active_prompt_cache_conversation(
                    &state.prompt_cache_conversation_cache,
                    prompt_cache_key,
                )
                .await;
            }
            if let Some(mut lease_guard) = lease_guard {
                lease_guard.disarm();
            }
            Err(err)
        }
    }
}

async fn allocate_proxy_invoke_id_serialized(
    state: &AppState,
    prompt_cache_key: Option<&str>,
) -> Result<PromptCacheInvokeIdAllocation> {
    if let Some(prompt_cache_key) = normalize_prompt_cache_key(prompt_cache_key) {
        let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
            .await;
        let (conversation_id, sequence) = 'reserve: {
            for recovery_attempt in 0..=1 {
                let cached_identity = {
                    let cache = state.prompt_cache_conversation_cache.lock().await;
                    cache
                        .identity_cache
                        .conversations
                        .get(prompt_cache_key)
                        .cloned()
                };
                let identity = if let Some(identity) = cached_identity {
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
                    let mut cache = state.prompt_cache_conversation_cache.lock().await;
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
                let conversation_id = identity.conversation_id;
                let next_sequence = identity.next_sequence;
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
                    state
                        .prompt_cache_conversation_cache
                        .lock()
                        .await
                        .identity_cache
                        .conversations
                        .remove(prompt_cache_key);
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
        let invoke_id = format!("{}{}", conversation_id, suffix);
        debug!(
            conversation_id = %conversation_id,
            invoke_id = %invoke_id,
            prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(prompt_cache_key),
            sequence,
            "allocated conversation-bound proxy invoke id"
        );
        drop(_write_permit);
        return Ok(PromptCacheInvokeIdAllocation {
            invoke_id,
            cache_update: PromptCacheInvokeIdCacheUpdate::Conversation {
                conversation_id,
                next_sequence: sequence.saturating_add(1),
            },
        });
    }

    let hour_key = Utc::now().timestamp().div_euclid(60 * 60);
    let cached_prefix = {
        let cache = state.prompt_cache_conversation_cache.lock().await;
        cache
            .identity_cache
            .unbound_prefix
            .as_ref()
            .filter(|prefix| prefix.hour_key == hour_key)
            .cloned()
    };
    let _write_permit = if cached_prefix.is_none() {
        Some(
            crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                .acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy,
                )
                .await,
        )
    } else {
        None
    };
    let mut prefix = if let Some(prefix) = cached_prefix {
        prefix
    } else {
        initialize_unbound_prompt_cache_prefix(&state.pool, hour_key).await?
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
    drop(_write_permit);
    Ok(PromptCacheInvokeIdAllocation {
        invoke_id,
        cache_update: PromptCacheInvokeIdCacheUpdate::Unbound { prefix },
    })
}

const PROMPT_CACHE_CONVERSATION_STATS_REFRESH_ATTEMPTS: usize = 3;
const PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY: usize = 400;

#[derive(Debug, FromRow)]
struct PromptCacheConversationStatsStagingRow {
    prompt_cache_key: String,
    generation: i64,
    source_max_invocation_id: i64,
    cursor_occurred_at: Option<String>,
    cursor_id: i64,
    accumulator_json: String,
    page_size: i64,
}

fn empty_prompt_cache_conversation_stats(
    prompt_cache_key: &str,
    conversation_id: &str,
) -> PromptCacheConversationStatsRow {
    PromptCacheConversationStatsRow {
        prompt_cache_key: prompt_cache_key.to_string(),
        conversation_id: conversation_id.to_string(),
        max_invoke_id: None,
        request_count: 0,
        success_count: 0,
        failure_count: 0,
        input_tokens: 0,
        output_tokens: 0,
        cache_input_tokens: 0,
        reported_cache_write_tokens: 0,
        reasoning_tokens: 0,
        total_tokens: 0,
        cost: 0.0,
        cost_input: 0.0,
        cost_cache_write: 0.0,
        cost_cache_read: 0.0,
        cost_output: 0.0,
        cost_reasoning: 0.0,
        first_invocation_at: None,
        last_invocation_at: None,
    }
}

fn merge_prompt_cache_conversation_invocation(
    stats: &mut PromptCacheConversationStatsRow,
    row: &PromptCacheConversationInvocationRow,
) {
    stats.request_count = stats.request_count.saturating_add(1);
    if invocation_status_is_success_like(row.status.as_deref(), row.error_message.as_deref()) {
        stats.success_count = stats.success_count.saturating_add(1);
    } else {
        stats.failure_count = stats.failure_count.saturating_add(1);
    }
    stats.input_tokens = stats
        .input_tokens
        .saturating_add(row.input_tokens.unwrap_or_default());
    stats.output_tokens = stats
        .output_tokens
        .saturating_add(row.output_tokens.unwrap_or_default());
    stats.cache_input_tokens = stats
        .cache_input_tokens
        .saturating_add(row.cache_input_tokens.unwrap_or_default());
    stats.reported_cache_write_tokens = stats
        .reported_cache_write_tokens
        .saturating_add(row.reported_cache_write_tokens.unwrap_or_default());
    stats.reasoning_tokens = stats
        .reasoning_tokens
        .saturating_add(row.reasoning_tokens.unwrap_or_default());
    stats.total_tokens = stats
        .total_tokens
        .saturating_add(row.total_tokens.unwrap_or_default());
    stats.cost += row.cost.unwrap_or_default();
    stats.cost_input += row.cost_input.unwrap_or_default();
    stats.cost_cache_write += row.cost_cache_write.unwrap_or_default();
    stats.cost_cache_read += row.cost_cache_read.unwrap_or_default();
    stats.cost_output += row.cost_output.unwrap_or_default();
    stats.cost_reasoning += row.cost_reasoning.unwrap_or_default();
    if stats
        .max_invoke_id
        .as_deref()
        .is_none_or(|value| row.invoke_id.as_str() > value)
    {
        stats.max_invoke_id = Some(row.invoke_id.clone());
    }
    if stats
        .first_invocation_at
        .as_deref()
        .is_none_or(|value| row.occurred_at.as_str() < value)
    {
        stats.first_invocation_at = Some(row.occurred_at.clone());
    }
    if stats
        .last_invocation_at
        .as_deref()
        .is_none_or(|value| row.occurred_at.as_str() > value)
    {
        stats.last_invocation_at = Some(row.occurred_at.clone());
    }
}

async fn refresh_prompt_cache_conversation_stats_bounded_page(
    pool: &Pool<Sqlite>,
    prompt_cache_key: &str,
    control: Option<&std::sync::Arc<crate::maintenance_store::PromptCacheMaterializationControl>>,
    expected_generation: Option<u64>,
) -> Result<PromptCacheStatsPageOutcome> {
    let _step = match prompt_cache_conversation_begin_control_step(control, expected_generation) {
        Ok(step) => step,
        Err(PromptCacheMaterializationControlStop::Disabled) => {
            return Ok(PromptCacheStatsPageOutcome::Disabled);
        }
        Err(PromptCacheMaterializationControlStop::GenerationChanged) => {
            return Ok(PromptCacheStatsPageOutcome::ControlGenerationChanged);
        }
        Err(PromptCacheMaterializationControlStop::Unavailable) => {
            return Ok(PromptCacheStatsPageOutcome::Unavailable);
        }
    };
    let deadline = Instant::now() + PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET;
    let connection =
        match tokio::time::timeout(PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET, pool.acquire())
            .await
        {
            Ok(connection) => connection?,
            Err(_) => {
                return Err(anyhow!(
                    "prompt-cache statistics page exceeded {}ms connection acquisition budget",
                    PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                ));
            }
        };
    let mut connection = PromptCacheConversationStatsProgressConnection {
        connection,
        progress_handler_installed: false,
    };
    let interrupted_by_budget = Arc::new(AtomicBool::new(false));
    {
        let interrupted_by_budget = interrupted_by_budget.clone();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!(
                "prompt-cache statistics page exceeded {}ms budget before SQLite handle acquisition",
                PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
            ));
        }
        let mut handle =
            match tokio::time::timeout(remaining, connection.connection.lock_handle()).await {
                Ok(handle) => handle
                    .context("failed to acquire SQLite handle for prompt-cache page budget")?,
                Err(_) => {
                    return Err(anyhow!(
                        "prompt-cache statistics page exceeded {}ms lock acquisition budget",
                        PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                    ));
                }
            };
        handle.set_progress_handler(PROMPT_CACHE_CONVERSATION_STATS_PROGRESS_OPS, move || {
            let within_budget = Instant::now() < deadline;
            if !within_budget {
                interrupted_by_budget.store(true, Ordering::Release);
            }
            within_budget
        });
        connection.progress_handler_installed = true;
    }

    macro_rules! budgeted_query {
        ($future:expr) => {{
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                connection.close_on_drop();
                return Err(anyhow!(
                    "prompt-cache statistics page exceeded {}ms budget before the next SQLite operation",
                    PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                ));
            }
            match tokio::time::timeout(remaining, $future).await {
                Ok(Ok(value)) => value,
                Ok(Err(error)) if interrupted_by_budget.load(Ordering::Acquire) => {
                    connection.close_on_drop();
                    return Err(anyhow!(
                        "prompt-cache statistics page exceeded {}ms execution budget: {error}",
                        PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                    ));
                }
                Ok(Err(error)) => {
                    connection.close_on_drop();
                    return Err(error.into());
                }
                Err(_) => {
                    connection.close_on_drop();
                    return Err(anyhow!(
                        "prompt-cache statistics page exceeded {}ms wall-clock budget",
                        PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                    ));
                }
            }
        }};
    }

    let queue_generation = budgeted_query!(
        sqlx::query_scalar::<_, i64>(&format!(
            "SELECT generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key = ?1"
        ))
        .bind(prompt_cache_key)
        .fetch_optional(&mut *connection.connection)
    );
    let clock_generation = budgeted_query!(
        sqlx::query_scalar::<_, i64>(&format!(
            "SELECT generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_GENERATION_CLOCK_TABLE} WHERE prompt_cache_key = ?1"
        ))
        .bind(prompt_cache_key)
        .fetch_optional(&mut *connection.connection)
    );
    let conversation_id = budgeted_query!(
        sqlx::query_scalar::<_, String>(
            "SELECT conversation_id FROM prompt_cache_conversations WHERE prompt_cache_key = ?1",
        )
        .bind(prompt_cache_key)
        .fetch_optional(&mut *connection.connection)
    );
    let Some(conversation_id) = conversation_id else {
        match tokio::time::timeout(
            PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET,
            connection.connection.lock_handle(),
        )
        .await
        {
            Ok(Ok(mut handle)) => {
                handle.remove_progress_handler();
                connection.progress_handler_installed = false;
                return Ok(PromptCacheStatsPageOutcome::Complete);
            }
            Ok(Err(error)) => {
                return Err(error.into());
            }
            Err(_) => {
                return Err(anyhow!(
                    "prompt-cache statistics page exceeded {}ms progress-handler cleanup budget",
                    PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                ));
            }
        }
    };
    let expected_generation = queue_generation.or(clock_generation).unwrap_or(0);
    let staging = budgeted_query!(
        sqlx::query_as::<_, PromptCacheConversationStatsStagingRow>(&format!(
            "SELECT prompt_cache_key,generation,source_max_invocation_id,cursor_occurred_at,cursor_id,accumulator_json,page_size FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} WHERE prompt_cache_key = ?1"
        ))
        .bind(prompt_cache_key)
        .fetch_optional(&mut *connection.connection)
    );
    let source_generation_changed = staging
        .as_ref()
        .is_some_and(|staging| staging.generation != expected_generation);
    let staging = match staging {
        Some(staging) if staging.generation == expected_generation => staging,
        _ => {
            let source_max_invocation_id = budgeted_query!(
                sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
                    .fetch_one(&mut *connection.connection)
            )
            .unwrap_or_default();
            let accumulator =
                empty_prompt_cache_conversation_stats(prompt_cache_key, &conversation_id);
            let accumulator_json = serde_json::to_string(&accumulator)?;
            budgeted_query!(
                sqlx::query(&format!(
                    "INSERT INTO {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} (prompt_cache_key,generation,source_max_invocation_id,cursor_occurred_at,cursor_id,accumulator_json,page_size,updated_at) VALUES (?1,?2,?3,NULL,0,?4,?5,STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(prompt_cache_key) DO UPDATE SET generation=excluded.generation,source_max_invocation_id=excluded.source_max_invocation_id,cursor_occurred_at=NULL,cursor_id=0,accumulator_json=excluded.accumulator_json,page_size=excluded.page_size,updated_at=excluded.updated_at"
                ))
                .bind(prompt_cache_key)
                .bind(expected_generation)
                .bind(source_max_invocation_id)
                .bind(accumulator_json)
                .bind(PROMPT_CACHE_CONVERSATION_STATS_PAGE_INITIAL_SIZE)
                .execute(&mut *connection.connection)
            );
            PromptCacheConversationStatsStagingRow {
                prompt_cache_key: prompt_cache_key.to_string(),
                generation: expected_generation,
                source_max_invocation_id,
                cursor_occurred_at: None,
                cursor_id: 0,
                accumulator_json: serde_json::to_string(&empty_prompt_cache_conversation_stats(
                    prompt_cache_key,
                    &conversation_id,
                ))?,
                page_size: PROMPT_CACHE_CONVERSATION_STATS_PAGE_INITIAL_SIZE,
            }
        }
    };
    if source_generation_changed {
        connection.close_on_drop();
        return Ok(PromptCacheStatsPageOutcome::GenerationChanged);
    }
    let mut accumulator: PromptCacheConversationStatsRow =
        serde_json::from_str(&staging.accumulator_json).with_context(|| {
            format!(
                "invalid prompt-cache statistics staging accumulator for {}",
                staging.prompt_cache_key
            )
        })?;
    let page_size = staging
        .page_size
        .clamp(1, PROMPT_CACHE_CONVERSATION_BACKFILL_PAGE_SIZE as i64);
    let invocation_key_expr = invocation_prompt_cache_key_expr_sql("i");
    let page_sql = format!(
        "SELECT i.id,i.invoke_id,i.occurred_at,i.status,i.error_message,i.input_tokens,i.output_tokens,i.cache_input_tokens,i.reported_cache_write_tokens,i.reasoning_tokens,i.total_tokens,i.cost,i.cost_input,i.cost_cache_write,i.cost_cache_read,i.cost_output,i.cost_reasoning FROM codex_invocations AS i WHERE {invocation_key_expr} = ?1 AND i.id <= ?2 AND (?3 IS NULL OR i.occurred_at > ?3 OR (i.occurred_at = ?3 AND i.id > ?4)) ORDER BY i.occurred_at ASC,i.id ASC LIMIT ?5"
    );
    let page_query = sqlx::query_as::<_, PromptCacheConversationInvocationRow>(&page_sql)
        .bind(prompt_cache_key)
        .bind(staging.source_max_invocation_id)
        .bind(staging.cursor_occurred_at.as_deref())
        .bind(staging.cursor_id)
        .bind(page_size);
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        connection.close_on_drop();
        return Err(anyhow!(
            "prompt-cache statistics page exceeded {}ms budget before invocation scan",
            PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
        ));
    }
    let page_result =
        match tokio::time::timeout(remaining, page_query.fetch_all(&mut *connection.connection))
            .await
        {
            Ok(result) => result,
            Err(_) => {
                connection.close_on_drop();
                return Err(anyhow!(
                    "prompt-cache statistics page exceeded {}ms wall-clock budget",
                    PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
                ));
            }
        };
    match tokio::time::timeout(
        PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET,
        connection.connection.lock_handle(),
    )
    .await
    {
        Ok(Ok(mut handle)) => {
            handle.remove_progress_handler();
            connection.progress_handler_installed = false;
        }
        Ok(Err(error)) => {
            return Err(error.into());
        }
        Err(_) => {
            return Err(anyhow!(
                "prompt-cache statistics page exceeded {}ms progress-handler cleanup budget",
                PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
            ));
        }
    }
    let page = match page_result {
        Ok(page) => page,
        Err(error) if interrupted_by_budget.load(Ordering::Acquire) => {
            let reduced_page_size = (page_size / 2).max(1);
            sqlx::query(&format!(
                "UPDATE {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} SET page_size = ?1, updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE prompt_cache_key = ?2 AND generation = ?3"
            ))
            .bind(reduced_page_size)
            .bind(prompt_cache_key)
            .bind(expected_generation)
            .execute(&mut *connection.connection)
            .await?;
            return Err(anyhow!(
                "prompt-cache statistics page exceeded {}ms execution budget: {error}",
                PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
            ));
        }
        Err(error) => return Err(error.into()),
    };
    for row in &page {
        merge_prompt_cache_conversation_invocation(&mut accumulator, row);
    }
    let mut tx = connection.connection.begin().await?;
    let current_generation = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key = ?1"
    ))
    .bind(prompt_cache_key)
    .fetch_optional(&mut *tx)
    .await?;
    if current_generation != queue_generation {
        let next_generation = current_generation.unwrap_or(0);
        let source_max_invocation_id =
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
                .fetch_one(&mut *tx)
                .await?
                .unwrap_or_default();
        let reset = serde_json::to_string(&empty_prompt_cache_conversation_stats(
            prompt_cache_key,
            &conversation_id,
        ))?;
        sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} SET generation=?1,source_max_invocation_id=?2,cursor_occurred_at=NULL,cursor_id=0,accumulator_json=?3,page_size=?4,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE prompt_cache_key=?5"
        ))
        .bind(next_generation)
        .bind(source_max_invocation_id)
        .bind(reset)
        .bind(PROMPT_CACHE_CONVERSATION_STATS_PAGE_INITIAL_SIZE)
        .bind(prompt_cache_key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(PromptCacheStatsPageOutcome::GenerationChanged);
    }
    if page.len() as i64 >= page_size {
        let last = page.last().expect("non-empty full prompt-cache page");
        let accumulator_json = serde_json::to_string(&accumulator)?;
        sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} SET cursor_occurred_at=?1,cursor_id=?2,accumulator_json=?3,page_size=?4,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE prompt_cache_key=?5 AND generation=?6"
        ))
        .bind(&last.occurred_at)
        .bind(last.id)
        .bind(accumulator_json)
        .bind(page_size)
        .bind(prompt_cache_key)
        .bind(expected_generation)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(PromptCacheStatsPageOutcome::Pending);
    }
    let max_sequence = accumulator.max_invoke_id.as_deref().and_then(|invoke_id| {
        let suffix = invoke_id_suffix(invoke_id, &accumulator.conversation_id)?;
        decode_prompt_cache_conversation_sequence(suffix)
    });
    sqlx::query(
        "UPDATE prompt_cache_conversations SET last_invoke_sequence=MAX(last_invoke_sequence,COALESCE(?1,-1)),request_count=?2,success_count=?3,failure_count=?4,input_tokens=?5,output_tokens=?6,cache_input_tokens=?7,reported_cache_write_tokens=?8,reasoning_tokens=?9,total_tokens=?10,cost=?11,cost_input=?12,cost_cache_write=?13,cost_cache_read=?14,cost_output=?15,cost_reasoning=?16,first_invocation_at=?17,last_invocation_at=?18,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE prompt_cache_key=?19",
    )
    .bind(max_sequence.map(i64::from))
    .bind(accumulator.request_count)
    .bind(accumulator.success_count)
    .bind(accumulator.failure_count)
    .bind(accumulator.input_tokens)
    .bind(accumulator.output_tokens)
    .bind(accumulator.cache_input_tokens)
    .bind(accumulator.reported_cache_write_tokens)
    .bind(accumulator.reasoning_tokens)
    .bind(accumulator.total_tokens)
    .bind(accumulator.cost)
    .bind(accumulator.cost_input)
    .bind(accumulator.cost_cache_write)
    .bind(accumulator.cost_cache_read)
    .bind(accumulator.cost_output)
    .bind(accumulator.cost_reasoning)
    .bind(accumulator.first_invocation_at.as_deref())
    .bind(accumulator.last_invocation_at.as_deref())
    .bind(prompt_cache_key)
    .execute(&mut *tx)
    .await?;
    if let Some(generation) = queue_generation {
        sqlx::query(&format!(
            "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key=?1 AND generation=?2"
        ))
        .bind(prompt_cache_key)
        .bind(generation)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(&format!(
        "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} WHERE prompt_cache_key=?1 AND generation=?2"
    ))
    .bind(prompt_cache_key)
    .bind(expected_generation)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(PromptCacheStatsPageOutcome::Complete)
}

fn prompt_cache_statistics_budget_error(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .contains("prompt-cache statistics query exceeded")
        || error
            .to_string()
            .contains("prompt-cache statistics page exceeded")
}

pub(crate) async fn refresh_prompt_cache_conversation_stats(
    pool: &Pool<Sqlite>,
    prompt_cache_keys: &HashSet<String>,
) -> Result<usize> {
    mark_prompt_cache_conversation_stats_stale(pool).await?;
    let mut last_error = None;
    for attempt in 1..=PROMPT_CACHE_CONVERSATION_STATS_REFRESH_ATTEMPTS {
        match refresh_prompt_cache_conversation_stats_once(pool, prompt_cache_keys).await {
            Ok(refreshed) => {
                let mut connection = pool.acquire().await?;
                let pending =
                    prompt_cache_conversation_refresh_queue_has_rows_on_connection(&mut connection)
                        .await
                        .unwrap_or(true);
                if pending {
                    mark_prompt_cache_conversation_stats_stale(pool).await?;
                } else {
                    mark_prompt_cache_conversation_stats_fresh(pool).await?;
                }
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
    for prompt_cache_keys in prompt_cache_keys
        .iter()
        .take(PROMPT_CACHE_CONVERSATION_STATS_MAX_KEYS_PER_QUERY)
    {
        match refresh_prompt_cache_conversation_stats_bounded_page(
            pool,
            prompt_cache_keys,
            None,
            None,
        )
        .await?
        {
            PromptCacheStatsPageOutcome::Complete => refreshed += 1,
            PromptCacheStatsPageOutcome::Pending
            | PromptCacheStatsPageOutcome::GenerationChanged
            | PromptCacheStatsPageOutcome::BudgetExhausted
            | PromptCacheStatsPageOutcome::Disabled
            | PromptCacheStatsPageOutcome::ControlGenerationChanged
            | PromptCacheStatsPageOutcome::Unavailable => {}
        }
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
    let deadline = Instant::now() + PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET;
    let interrupted_by_budget = Arc::new(AtomicBool::new(false));
    {
        let interrupted_by_budget = interrupted_by_budget.clone();
        let mut handle = connection
            .lock_handle()
            .await
            .context("failed to acquire SQLite handle for prompt-cache query budget")?;
        handle.set_progress_handler(PROMPT_CACHE_CONVERSATION_STATS_PROGRESS_OPS, move || {
            let within_budget = Instant::now() < deadline;
            if !within_budget {
                interrupted_by_budget.store(true, Ordering::Release);
            }
            within_budget
        });
    }
    let stats_rows_result = stats_query.fetch_all(&mut *connection).await;
    connection
        .lock_handle()
        .await
        .context("failed to reacquire SQLite handle after prompt-cache query")?
        .remove_progress_handler();
    let stats_rows = match stats_rows_result {
        Ok(stats_rows) => stats_rows,
        Err(error) if interrupted_by_budget.load(Ordering::Acquire) => {
            return Err(anyhow!(
                "prompt-cache statistics query exceeded {}ms execution budget: {error}",
                PROMPT_CACHE_CONVERSATION_STATS_QUERY_BUDGET.as_millis()
            ));
        }
        Err(error) => return Err(error.into()),
    };
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
        let queue_generations =
            load_prompt_cache_conversation_stats_queue_generations_on_connection(
                tx.as_mut(),
                &prompt_cache_key_refs,
            )
            .await?;
        refreshed += refresh_prompt_cache_conversation_stats_on_connection(
            tx.as_mut(),
            &prompt_cache_key_refs,
        )
        .await?;
        clear_prompt_cache_conversation_stats_refresh_queue_at_generation_on_connection(
            tx.as_mut(),
            &queue_generations,
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

async fn load_prompt_cache_conversation_stats_queue_generations_on_connection(
    connection: &mut SqliteConnection,
    prompt_cache_keys: &[&str],
) -> Result<Vec<(String, i64)>> {
    if prompt_cache_keys.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = std::iter::repeat_n("?", prompt_cache_keys.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT prompt_cache_key, generation FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key IN ({placeholders})"
    );
    let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
    for prompt_cache_key in prompt_cache_keys {
        query = query.bind(*prompt_cache_key);
    }
    Ok(query.fetch_all(&mut *connection).await?)
}

async fn clear_prompt_cache_conversation_stats_refresh_queue_at_generation_on_connection(
    connection: &mut SqliteConnection,
    queue_generations: &[(String, i64)],
) -> Result<()> {
    for (prompt_cache_key, generation) in queue_generations {
        sqlx::query(&format!(
            "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key = ?1 AND generation = ?2"
        ))
        .bind(prompt_cache_key)
        .bind(generation)
        .execute(&mut *connection)
        .await?;
    }
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
    Ok(cleanup_orphan_prompt_cache_conversations_with_active_keys(
        pool,
        dry_run,
        &HashSet::new(),
        None,
    )
    .await?
    .released)
}

#[derive(Debug, Default)]
struct PromptCacheConversationOrphanCleanupResult {
    released: usize,
    deleted_prompt_cache_identities: Vec<(String, String)>,
}

async fn cleanup_orphan_prompt_cache_conversations_with_active_keys(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    active_prompt_cache_keys: &HashSet<String>,
    cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
) -> Result<PromptCacheConversationOrphanCleanupResult> {
    async fn bounded_query<T, E, F>(future: F) -> Result<T>
    where
        E: Into<anyhow::Error>,
        F: std::future::Future<Output = std::result::Result<T, E>>,
    {
        let value = if let Some(remaining) = crate::maintenance::retention_run_remaining_budget() {
            tokio::time::timeout(remaining, future)
                .await
                .map_err(|_| anyhow!(PROMPT_CACHE_ORPHAN_CLEANUP_BUDGET_EXPIRED))?
                .map_err(Into::into)?
        } else {
            future.await.map_err(Into::into)?
        };
        Ok(value)
    }

    let prompt_key_expr = invocation_prompt_cache_key_expr_sql("i");
    let mut result = PromptCacheConversationOrphanCleanupResult::default();
    let cursor_key = bounded_query(sqlx::query_scalar::<_, Option<String>>(&format!(
        "SELECT cursor_key FROM {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE} WHERE scope='prompt_cache_conversations'"
    ))
    .fetch_one(pool))
    .await?;
    let candidates = bounded_query(sqlx::query_as::<_, (String, String)>(&format!(
        "SELECT prompt_cache_key,conversation_id FROM prompt_cache_conversations WHERE (last_invocation_at IS NOT NULL OR created_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ','now','-{PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES} minutes')) AND updated_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ','now','-{PROMPT_CACHE_CONVERSATION_ORPHAN_GRACE_MINUTES} minutes') AND (?1 IS NULL OR prompt_cache_key > ?1) ORDER BY prompt_cache_key LIMIT {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_PAGE_SIZE}"
    ))
    .bind(cursor_key.as_deref())
    .fetch_all(pool))
    .await?;
    if candidates.is_empty() {
        bounded_query(sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE} SET cursor_key=NULL,epoch=epoch+1,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE scope='prompt_cache_conversations'"
        ))
        .execute(pool))
        .await?;
        return Ok(result);
    }
    let mut eligible = Vec::new();
    for (prompt_cache_key, conversation_id) in &candidates {
        if active_prompt_cache_keys.contains(prompt_cache_key) {
            continue;
        }
        let key_referenced = bounded_query(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT EXISTS(SELECT 1 FROM codex_invocations AS i WHERE {prompt_key_expr} = ?1)"
            ))
            .bind(prompt_cache_key)
            .fetch_one(pool),
        )
        .await?
            != 0;
        let invoke_id_referenced = bounded_query(sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM codex_invocations WHERE length(invoke_id)=?1 AND invoke_id>=?2 AND invoke_id < (?2 || '['))",
        )
        .bind(PROXY_INVOKE_ID_LENGTH as i64)
        .bind(conversation_id)
        .fetch_one(pool))
        .await?
            != 0;
        if !key_referenced && !invoke_id_referenced {
            eligible.push((prompt_cache_key.clone(), conversation_id.clone()));
        }
    }
    if dry_run {
        result.released = eligible.len();
        let full_page = candidates.len() == PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_PAGE_SIZE;
        let next_cursor = full_page.then(|| {
            candidates
                .last()
                .expect("non-empty candidate page")
                .0
                .clone()
        });
        bounded_query(sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE} SET cursor_key=?1,epoch=epoch+CASE WHEN ?2 THEN 0 ELSE 1 END,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE scope='prompt_cache_conversations'"
        ))
        .bind(next_cursor)
        .bind(full_page)
        .execute(pool))
        .await?;
    } else {
        // Exact reference probes run without the cache mutex. Reacquire it only for the
        // short delete transaction so a lease that arrived during those probes wins over
        // deletion without blocking request admission on the slow probes.
        let cache_guard = match cache {
            Some(cache) => Some(
                if let Some(remaining) = crate::maintenance::retention_run_remaining_budget() {
                    tokio::time::timeout(remaining, cache.lock())
                        .await
                        .map_err(|_| anyhow!(PROMPT_CACHE_ORPHAN_CLEANUP_BUDGET_EXPIRED))?
                } else {
                    cache.lock().await
                },
            ),
            None => None,
        };
        let mut tx = bounded_query(pool.begin()).await?;
        for (prompt_cache_key, conversation_id) in &eligible {
            if cache_guard.as_ref().is_some_and(|state| {
                state
                    .identity_cache
                    .active_prompt_cache_keys
                    .contains_key(prompt_cache_key)
            }) {
                continue;
            }
            let deleted = bounded_query(sqlx::query(
                "DELETE FROM prompt_cache_conversations WHERE prompt_cache_key=?1 AND conversation_id=?2 AND (last_invocation_at IS NOT NULL OR created_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ','now','-5 minutes')) AND updated_at < STRFTIME('%Y-%m-%dT%H:%M:%fZ','now','-5 minutes') AND NOT EXISTS (SELECT 1 FROM codex_invocations AS i WHERE CASE WHEN json_valid(i.payload) THEN TRIM(CAST(json_extract(i.payload,'$.promptCacheKey') AS TEXT)) END = ?1) AND NOT EXISTS (SELECT 1 FROM codex_invocations WHERE length(invoke_id)=?3 AND invoke_id>=?2 AND invoke_id < (?2 || '['))",
            )
            .bind(prompt_cache_key)
            .bind(conversation_id)
            .bind(PROXY_INVOKE_ID_LENGTH as i64)
            .execute(&mut *tx))
            .await?
            .rows_affected();
            if deleted > 0 {
                bounded_query(sqlx::query(&format!(
                    "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_QUEUE_TABLE} WHERE prompt_cache_key=?1"
                ))
                .bind(prompt_cache_key)
                .execute(&mut *tx))
                .await?;
                bounded_query(sqlx::query(&format!(
                    "DELETE FROM {PROMPT_CACHE_CONVERSATIONS_STATS_REFRESH_STAGING_TABLE} WHERE prompt_cache_key=?1"
                ))
                .bind(prompt_cache_key)
                .execute(&mut *tx))
                .await?;
                result.released = result.released.saturating_add(1);
                result
                    .deleted_prompt_cache_identities
                    .push((prompt_cache_key.clone(), conversation_id.clone()));
            }
        }
        let full_page = candidates.len() == PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_PAGE_SIZE;
        let next_cursor = full_page.then(|| {
            candidates
                .last()
                .expect("non-empty candidate page")
                .0
                .clone()
        });
        bounded_query(sqlx::query(&format!(
            "UPDATE {PROMPT_CACHE_CONVERSATION_ORPHAN_CLEANUP_STATE_TABLE} SET cursor_key=?1,epoch=epoch+CASE WHEN ?2 THEN 0 ELSE 1 END,updated_at=STRFTIME('%Y-%m-%dT%H:%M:%fZ','now') WHERE scope='prompt_cache_conversations'"
        ))
        .bind(next_cursor)
        .bind(full_page)
        .execute(&mut *tx))
        .await?;
        bounded_query(tx.commit()).await?;
    }
    Ok(result)
}

pub(crate) async fn cleanup_orphan_prompt_cache_conversations_with_cache(
    pool: &Pool<Sqlite>,
    dry_run: bool,
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) -> Result<usize> {
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
        dry_run,
        &active_prompt_cache_keys,
        Some(cache),
    )
    .await?;
    if !dry_run && cleanup.released > 0 {
        let mut cache_state = cache.lock().await;
        for (prompt_cache_key, conversation_id) in cleanup.deleted_prompt_cache_identities {
            if cache_state
                .identity_cache
                .conversations
                .get(&prompt_cache_key)
                .is_some_and(|identity| identity.conversation_id == conversation_id)
            {
                cache_state
                    .identity_cache
                    .conversations
                    .remove(&prompt_cache_key);
            }
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
