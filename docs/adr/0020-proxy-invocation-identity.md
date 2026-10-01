# Proxy Invocation Identity and Prompt-Cache Conversation Master

## Status

Accepted

Proxy invocation identity is a backend-owned contract. A prompt-cache key maps to one durable
conversation master row and receives a six-character conversation prefix. Each invocation appends a
four-character ordered base-31 sequence. Invocations without a prompt-cache key use an in-memory
six-character prefix scoped to the current UTC hour and the same ordered suffix. Allocation is
serialized per normalized prompt-cache key; a cache miss recovers the next sequence from SQLite
rather than confirming every generated invocation against the database. Unbound prefix
initialization uses a separate process-local namespace, while hot suffix allocation uses its
short-lived cache entry lock.

The conversation master stores the normalized prompt-cache key, identity, aggregate invocation
statistics, and first/last invocation times. Statistics are refreshed after batched terminal and
derived writes. A schema marker makes the legacy prompt-cache-key backfill idempotent, and retention
removes masters whose retained invocation rows no longer exist so released prefixes are not treated
as a permanent blacklist.

## Consequences

- New HTTP and WebSocket proxy invocation IDs are exactly ten characters from the existing custom
  alphabet.
- Existing invocation rows are read as historical input during backfill; their IDs are not rewritten.
- Conversation identity allocation can fail explicitly on bounded candidate collisions or sequence
  exhaustion, and those failures are observable through structured logs.
- Public API fields and Dashboard consumers remain a separate follow-up boundary.
