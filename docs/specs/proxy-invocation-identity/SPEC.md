# Proxy Invocation Identity

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: Proxy request identifiers must remain compact while preserving conversation-level ordering and recoverability across process restarts.
- In scope: Backend HTTP and WebSocket proxy allocation, the prompt-cache conversation master, SQLite migration/backfill, delayed statistics, retention, and diagnostics.
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

### REQ-PII-003

- The allocator MUST keep normal next-sequence state in memory, recover it from the master row and retained invocation IDs after a cache miss, and avoid a database uniqueness confirmation for every invocation.
- A concurrent prompt-key creation race MUST recover the already persisted master row instead of creating a duplicate identity.

### REQ-PII-004

- Schema creation and legacy prompt-cache-key backfill MUST be idempotent and separately observable from aggregate statistics refresh.
- Terminal and derived batch writes MUST persist the invocation, enqueue affected prompt-cache keys, and invalidate aggregate freshness without synchronously scanning retained history. The ordered background materialization task MUST refresh identities and aggregate statistics from the durable queue and advance its cursor only after each committed transaction.
- A 400-key logical materialization page MAY be split into adaptive 32..400-key micro-batches. Prompt-cache pressure MUST yield only between committed micro-batches; a started micro-batch runs to commit or explicit failure.
- Aggregate reads MUST use the existing unavailable contract until identity coverage, aggregate freshness, complete migration phase, and an empty durable refresh queue are all satisfied. Retention MUST release masters whose retained invocation rows have been removed without maintaining a permanent released-ID blacklist.

### REQ-PII-005

- Allocation, cache recovery, migration/backfill, delayed statistics refresh, retention release, sequence exhaustion, and bounded allocation errors MUST emit diagnostic logs without logging raw prompt-cache keys. Materialization logs MUST include phase, cursor, scanned/updated counts, batch size and duration, and pressure defer/failure state.

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

## Related ADRs

- [`../../adr/0020-proxy-invocation-identity.md`](../../adr/0020-proxy-invocation-identity.md)
- [`../../adr/0021-prompt-cache-background-materialization.md`](../../adr/0021-prompt-cache-background-materialization.md)
- [`../../adr/0022-prompt-cache-adaptive-materialization.md`](../../adr/0022-prompt-cache-adaptive-materialization.md)

## Visual Evidence

- None

## References

- `./IMPLEMENTATION.md`
- `./HISTORY.md`
