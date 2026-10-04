# Proxy Invocation Identity

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: Proxy request identifiers must remain compact while preserving conversation-level ordering and recoverability across process restarts.
- In scope: Backend HTTP proxy allocation, historical WebSocket invocation identity reads, the prompt-cache conversation master, SQLite migration/backfill, delayed statistics, retention, and diagnostics.
- Out of scope: Public API response fields, Dashboard/UI consumers, and frontend identifier presentation.

## Terms and Interfaces

- `conversation_id`: A six-character identifier generated from the existing 31-character proxy alphabet.
- `invoke_id`: A ten-character identifier consisting of a six-character prefix and a four-character ordered sequence.
- `prompt-cache conversation master`: The durable `prompt_cache_conversations` row keyed by one normalized `prompt_cache_key`.
- Interface: `src/prompt_cache_conversations.rs` and the proxy capture/runtime persistence paths.

## Requirements

### REQ-PII-001

- The system MUST generate exactly ten-character proxy invocation IDs from the existing custom alphabet.
- A prompt-cache invocation MUST use its master `conversation_id` as the first six characters and an ordered four-character base-31 sequence as the suffix.
- An invocation without a prompt-cache key MUST use a generated six-character prefix scoped to the current UTC hour and an ordered four-character suffix.

### REQ-PII-002

- The system MUST persist one prompt-cache conversation master per normalized prompt-cache key, including its conversation ID, aggregate invocation counts, token totals, cost totals, and first/last invocation timestamps.
- The master conversation ID MUST be checked against existing master IDs before insertion, with bounded candidate retries and an explicit failure after exhaustion.
- A newly generated conversation prefix MUST exclude process-issued unbound prefixes as well as existing conversation masters and invocation prefixes.

### REQ-PII-003

- The allocator MUST keep normal next-sequence state in memory, recover it from the master row and retained invocation IDs after a cache miss, and avoid a database uniqueness confirmation for every invocation.
- A concurrent prompt-key creation race MUST recover the already persisted master row instead of creating a duplicate identity.
- Allocation MUST serialize only the normalized prompt-cache key that is being reserved; unrelated keys MUST NOT wait on the identity cache while identity or sequence SQL is in flight. Active prompt-cache references MUST be registered before an allocation wait and released on failure or cancellation.
- Unbound prefix initialization MUST use a separate process-local namespace lock; hot suffix allocation MUST use only its short-lived per-cache lock.

### REQ-PII-004

- Schema creation and legacy prompt-cache-key backfill MUST be idempotent and separately observable from aggregate statistics refresh.
- Terminal and derived batch writes MUST persist the invocation, enqueue affected prompt-cache keys, and invalidate aggregate freshness without synchronously scanning retained history. The ordered background materialization task MUST refresh identities and aggregate statistics from the durable queue and advance its cursor only after each committed transaction.
- A 400-key logical materialization page MAY be split into adaptive 32..400-key micro-batches. Prompt-cache pressure MUST yield only between committed micro-batches; a started micro-batch runs to commit or explicit failure.
- Identity discovery MUST commit its identity rows, key cursor, and identity-key count together without scanning statistics. Statistics rebuild and queue drain MUST own resumable statistics pages. A statistics rebuild MUST commit each completed conversation's aggregate, generation-bound queue/staging cleanup, and continuous outer key cursor in the same transaction. An unfinished conversation MUST NOT advance that cursor. Queue drain MUST use generation-bound queue removal as its completion checkpoint.
- Statistics pages with unchanged sources MUST continue within the existing run and query budgets, checking control and interactive pressure between committed pages. Repeated pages of one conversation MUST consume only one visited-key allowance per run. A completed continuous prefix MUST NOT restart when a later conversation yields; source changes after publication MUST enqueue only the affected conversation.
- Aggregate reads MUST use the existing unavailable contract until identity coverage, aggregate freshness, complete migration phase, and an empty durable refresh queue are all satisfied. Retention MUST release masters whose retained invocation rows have been removed without maintaining a permanent released-ID blacklist.

### REQ-PII-005

- Allocation, cache recovery, migration/backfill, delayed statistics refresh, retention release, sequence exhaustion, and bounded allocation errors MUST emit diagnostic logs without logging raw prompt-cache keys. Materialization logs MUST include phase, cursor, scanned/updated counts, batch size and duration, and pressure defer/failure state.

### REQ-PII-006

- The prompt-cache materialization task MUST use its `managed_tasks` row in the maintenance database as the sole enablement authority. Its scheduler checkpoint MUST be updated in the same maintenance-database transaction. The legacy business-database `startup_backfill_progress.enabled` value MUST NOT control execution after migration or be mirrored by the task; it may seed a managed control once only when no corresponding maintenance task row existed before registry seeding.
- The dedicated materialization PATCH and managed-task PATCH MUST publish the same committed control state. A changed enablement value advances an in-process control generation; a repeated value does not. A step registered before a control change may finish, and later steps from the stale generation MUST stop before opening their SQLite transaction.
- Statistics pages MUST distinguish pending continuation, source-generation change, budget exhaustion, actual disablement, priority yield, and unavailable maintenance control. Only a complete page may advance its outer key cursor or publish complete statistics. Incomplete pages MUST preserve committed staging progress and MUST NOT be reported as `operator_disabled` or returned as a complete aggregate.
- A committed pending page MAY continue within the same run's remaining budget. Continuation deferred by a generation change or exhausted bounded work budget MUST use the existing 15-second follow-up. Priority and database pressure MUST retain their qualification/deadline path. Same-generation wakes and stale run results MUST preserve the durable delay. Actual disablement clears pending scheduler entries; only a new enablement generation may wake that task. Maintenance control failure MUST preserve the last committed in-memory state and report unavailable when no trusted state exists.
- Existing status and control response fields remain compatible. The existing `deferReason` string distinguishes `stats_page_pending`, `stats_generation_changed`, `stats_budget_exhausted`, `coordinator_priority`, `operator_disabled`, and `maintenance_database_unavailable`.
- Run `scanned` MUST count actually visited conversation keys, and `updated` MUST count committed identity creation or complete aggregate publication. Identity progress counters MUST retain their existing meaning. ETA MUST be unavailable while statistics rebuild or queue drain is incomplete and zero only after materialization is complete.

### REQ-PII-007

- The synchronous live Prompt working-set update trigger MUST run only for source columns that can affect its key, scope, displayed status, activity timestamps, counts, tokens, or cost. A terminal write that updates only persistence timing MUST NOT recompute this projection.
- Inserts, deletes, and relevant source updates MUST retain the existing live-window and old/new-key reconciliation semantics, including updates assigning an unchanged value.
- Existing databases MUST receive the corrected trigger definitions through a transaction that also records a durable completion marker. Interrupted installation MUST roll back and remain retryable; repeated startup MUST preserve projection rows, historical data, control state, and materialization checkpoints without rebuilding rows solely to update trigger dependencies.

## Verification

### VER-PII-001

- Method: Rust unit and stateful SQLite allocator tests.
- covers: `REQ-PII-001`
- Pass condition: Conversation-bound IDs are six-plus-four characters with ordered suffixes, unbound IDs share the current hourly prefix, and sequence overflow returns an error.

### VER-PII-002

- Method: Stateful SQLite schema migration and statistics tests.
- covers: `REQ-PII-002`, `REQ-PII-004`
- Pass condition: Backfill creates valid master rows, materializes aggregate fields through resumable 400-key logical pages and adaptive 32..400-key transactions, reruns without duplication, and retention removes released orphan masters.

### VER-PII-003

- Method: Source inspection plus allocator recovery test after clearing the process cache.
- covers: `REQ-PII-003`
- Pass condition: The next sequence is recovered from persisted invocation history and no per-invocation existence query is present in the normal allocation path.

### VER-PII-004

- Method: Structured tracing call-site inspection and focused proxy compilation.
- covers: `REQ-PII-005`
- Pass condition: Diagnostic fields contain fingerprints or generated IDs, materialization progress exposes the required batch/defer fields, and raw prompt-cache keys are excluded from allocator and migration logs.

### VER-PII-005

- Method: Independent business/maintenance SQLite regression tests, scheduler-generation tests, upgrade compatibility checks, and a non-test Linux service-process replay.
- covers: `REQ-PII-006`
- Pass condition: Both contradictory legacy business enablement values follow only the maintenance control; pause/resume transactions publish atomically; multiple large conversations in one batch continue within the run budget and retain per-conversation checkpoints across yield/restart; completed prefixes are not rescanned; a final-page failure rolls back aggregate, queue/staging cleanup and outer cursor together; repeated same-generation wakes do not advance an unexpired pressure deadline or start duplicate work; priority yield resumes after the foreground writer completes; actual disablement waits for a new control generation; existing HTTP fields remain compatible; incomplete statistics ETA is null; and prompt-cache pages converge to exact complete statistics with an empty queue and no pure-wait run record.

### VER-PII-006

- Method: Production terminal-write regression, incremental-versus-rebuild projection comparison, interrupted legacy-trigger upgrade/reentry regression, older-reader compatibility, and the same three-round Linux service replay used by `VER-PII-005`.
- covers: `REQ-PII-007`
- Pass condition: The actual terminal timing follow-up performs no projection update; relevant mutations, key moves and removal match a source rebuild; failed marker installation rolls back the trigger DDL; successful and repeated startup preserve business rows; existing readers accept the upgraded database; and all probe and steady calls remain included in the unchanged online latency and completion gates.

### VER-PII-007

- Method: Stateful SQLite multi-key checkpoint, budget, failure, pause/restart, source-generation, queue-drain, and nullable-ETA regressions, plus a SHA-bound three-round Linux service replay with synthetic history and per-page event evidence.
- covers: `REQ-PII-004`, `REQ-PII-006`
- Pass condition: The candidate publishes each completed rebuild key exactly once, resumes the first unfinished key from committed staging, advances only a continuous prefix in the final-page transaction, services a changed key behind the prefix through queue drain without rescanning other completed keys, preserves queue-drain cursor independence, reports actual visited/published work and null/zero ETA at the correct phases, reaches an exact empty-queue/empty-staging complete state, and stays within the existing online p99 bounds.

## Related ADRs

- [`../../adr/0020-proxy-invocation-identity.md`](../../adr/0020-proxy-invocation-identity.md)
- [`../../adr/0021-prompt-cache-background-materialization.md`](../../adr/0021-prompt-cache-background-materialization.md)
- [`../../adr/0022-prompt-cache-adaptive-materialization.md`](../../adr/0022-prompt-cache-adaptive-materialization.md)
- [`../../adr/0023-task-operations-state-outside-main-database.md`](../../adr/0023-task-operations-state-outside-main-database.md)
- [`../../adr/0027-prompt-cache-materialization-step-boundaries.md`](../../adr/0027-prompt-cache-materialization-step-boundaries.md)
- [`../../adr/0028-prompt-cache-continuous-statistics-checkpoints.md`](../../adr/0028-prompt-cache-continuous-statistics-checkpoints.md)

## Visual Evidence

- None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
