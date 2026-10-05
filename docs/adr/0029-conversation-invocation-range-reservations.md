# Conversation Invocation Range Reservations

## Status

Accepted

The conversation-bound allocator currently reserves one sequence in SQLite for every invocation,
including cache hits. That contradicts the intended memory allocation path in
[Proxy Invocation Identity](0020-proxy-invocation-identity.md). Terminal persistence happens after
ID allocation and cannot by itself prevent reuse of an issued sequence after an unexpected restart.

Reserve conversation-bound Invocation Sequences in the existing conversation master before issuing
them, then allocate from each committed range in memory. The durable `last_invoke_sequence` becomes
the Reserved Invocation Ceiling, rather than the last issued sequence. Recovery starts above that
ceiling; unused reserved values that were not returned are deliberately skipped. Actual invocation
statistics remain independent of this ceiling. This refines the conversation-bound allocation
decision in ADR 0020.

## Refill Policy

- Each reservation grants 64 sequences, except a smaller final range at the four-character suffix
  limit. Exhaustion never wraps the sequence or increases the Invocation ID length.
- Remaining capacity means unissued sequences in the current range. With no standby range or refill
  already in flight, fewer than 32 remaining sequences actively trigger a refill; fewer than 48 make
  another conversation eligible to join that refill batch. Exactly 32 does not actively trigger;
  exactly 48 does not join.
- One allocator-wide coordinator combines the triggering conversation with eligible conversations.
  Each conversation can hold at most one standby range and participate in at most one refill at a
  time. Combining reservations reduces commits; it does not remove the need to update each included
  conversation's durable ceiling.
- A batch publishes its standby ranges only after the database commit succeeds. Failed reservations
  cannot supply IDs; previously committed unused sequences remain available.
- If the current and standby ranges are empty while their shared refill batch is pending, allocation
  may wait for that batch for at most 100 ms. This is one total wait budget, not a new budget on each
  notification. A successful publication lets allocation resume; timeout or refill failure returns
  HTTP 503 with a structured diagnostic. Waiting does not issue a separate per-invocation database
  reservation, and a request timeout does not cancel the batch shared by other requests.
- Cache initialization recovers the durable ceiling and reserves a range. Ordinary allocation from
  a committed range performs only memory operations; database work belongs to initialization and
  refill. No new per-conversation files or conversation mapping table are needed for these ranges.

## Allocation Cache Capacity

Initialize allocation cache capacity to 128 immediately at startup. Initial database estimation is
asynchronous and does not delay request admission. Let `N` be the estimated distinct conversations
active in the preceding 48 hours; the target capacity is `min(4096, max(128, N))`. The minimum target
is 128 and the maximum is 4096; a full cache does not authorize exceeding that maximum.

Seed a bounded in-memory recent-activity index once from the conversation master's indexed
`last_invocation_at`, using a captured UTC cutoff. Fetch at most the 4096 most recently active
conversation identities and their timestamps, rather than counting every matching row:

```sql
SELECT conversation_id, last_invocation_at
FROM prompt_cache_conversations
     INDEXED BY idx_prompt_cache_conversations_last_invocation
WHERE last_invocation_at >= ?1
ORDER BY last_invocation_at DESC, conversation_id
LIMIT 4096;
```

This is a covering index range read with an early row limit. An outer `LIMIT` on `COUNT(*)` would
not bound the rows scanned. If the required index is unavailable, warn and retain the current
memory-derived target instead of falling back to a full-table scan. `updated_at` is not invocation
activity: reservation and statistics maintenance can change it independently of a new invocation.
Delayed statistics can affect the seed estimate; capacity sizing does not wait for materialization.

The recent-activity index stores only conversation identity and latest activity time, independently
of the allocation cache. Keep at most the 4096 most recently active identities. Each observed
conversation invocation refreshes its activity in memory, including invocations observed while the
seed query is pending. Merge seed rows by taking the later timestamp so startup history cannot
overwrite newer live activity. Cache eviction does not remove activity history; this history does
not reserve a conversation prefix or prevent its release by retention.

After merging the seed, derive the target from the in-memory index. Once per hour, expire activity
outside the rolling 48-hour window and recompute the target from that index only. Periodic resizing
does not query SQLite. Capped estimates are logged as lower bounds, not exact counts. The index is
an estimate for capacity and is not an authority for invocation statistics or ID uniqueness.

Sizing determines capacity; allocation entries still load on demand, without pre-reserving ranges
for seed identities. If the asynchronous seed fails, retain the current memory-derived target,
initially 128, warn with the reason, and continue tracking live activity. Hourly recomputation does
not retry the seed query. Log seed and resize source, cutoff, capped-count indication, old and new
targets, actual occupancy, elapsed time, and outcome without raw Prompt Cache Keys. Sizing and seed
work do not overlap or catch up missed intervals in a burst.

Shrink by retiring idle entries through the reservation-return policy below. Entries with active
invocations, allocations, or refills remain protected; a smaller target does not authorize clearing
their state. Complete any deferred shrink as those entries become idle. Log capacity changes and
deferred shrink counts so activity estimates and actual cache occupancy can be distinguished.

When the calculated target is full and every entry is protected, temporarily grow allocation
occupancy up to the 4096-entry maximum. At that maximum, a request needing an uncached conversation
waits for admission for at most 100 ms, then returns HTTP 503 if no slot becomes available. Admission
must reserve the slot atomically before loading identity or reserving a range. Existing cached
conversations continue allocating from their ranges; saturation does not introduce per-invocation
SQL fallback. Once entries become idle, retire them toward the calculated target. Log saturation,
admission timeout, temporary growth, and deferred contraction separately.

## Cache Eviction and Reservation Return

Cache eviction keeps the durable conversation master. Before retiring an idle entry, the unified
range manager makes a best-effort, time-bounded attempt to return its never-issued reservation tail.
This permits bounded caches without deliberately keeping every allocation entry until retention.

- Only entries with no active invocation, allocation, or refill in flight are eligible. Mark an
  entry as retiring under its per-conversation serialization before taking the return snapshot.
  Allocation, refill, and lifecycle cleanup must not race that transition. Database waits do not
  hold the global cache lock.
- Return only the contiguous unused tail owned by this entry, including an unused standby range.
  The return floor is the entry's highest allocated sequence, initialized to the boundary before
  its first reservation. Previously skipped ranges from another cache lifetime are not reclaimed.
  An allocated sequence remains spent even when its invocation is cancelled or has not persisted
  its terminal record.
- Use a single conditional update of the existing master, matching its conversation identity and
  the expected Reserved Invocation Ceiling before lowering the ceiling to the return floor. No
  per-invocation record scan or new mapping table is required. A confirmed commit is the only
  successful-return result; an unmatched condition cannot be treated as success.
- The total attempt budget is 100 ms and covers coordination, database admission, connection
  acquisition, execution, and commit. Busy, timeout, conditional-update mismatch, or failure produces
  a warning and retires the cache entry without retrying the return. Discarding memory never
  authorizes reuse of an unconfirmed range; subsequent initialization recovers the database's
  committed ceiling.
- Cancellation and rollback must be verified before permitting an old return operation to race a
  replacement entry. Merely abandoning an asynchronous database future is insufficient. Old
  callbacks must not publish ranges or mutate a replacement cache generation.
- The Reserved Invocation Ceiling is owned by the range manager. Statistics refresh must not
  overwrite it using a stale aggregate snapshot; migration establishes its initial recovery floor.

For example, an entry holding sequences 0 through 127 that has allocated only 0 through 19 may
return 20 through 127 by conditionally changing its durable ceiling from 127 to 19. If return fails,
the replacement entry recovers the durable ceiling and skips any reservations still outstanding.

Return diagnostics include the conversation identity, cache generation, expected ceiling, return
floor, attempted return count, elapsed time, and outcome or failure reason. They exclude the raw
Prompt Cache Key and distinguish an unconfirmed return from a confirmed successful return.

## Consequences

An unexpected restart may skip both the current range's unused sequences and its standby range.
This spends part of the finite suffix space in exchange for preventing reuse of issued IDs without
per-invocation durable writes. Cancellation does not return an issued sequence to the range.
Reservation follows the existing conversation retention lifecycle; it does not create a permanent
ID blacklist after retained records and their master are released.

Structured allocation diagnostics must distinguish range reservation, range publication, recovery,
retry or exhaustion, and lifecycle release. They should identify the conversation and range bounds
without exposing the raw Prompt Cache Key. Hot allocation diagnostics must not introduce a
synchronous per-invocation persistence requirement.

The durable hourly state for Unbound Invocations and its participation in the same range manager
are defined in [Durable Hourly Invocation Prefixes](0030-durable-hourly-invocation-prefixes.md).
