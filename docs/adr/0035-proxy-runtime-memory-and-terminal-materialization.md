# ADR 0035: Proxy runtime memory state and terminal materialization

## Context

The proxy request path previously used the SQLite write coordinator for attempt creation, progress updates, response metadata, and terminal updates. A slow or contended SQLite writer could therefore add database admission latency to an otherwise active upstream request.

The proxy runtime also needs bounded live state, crash recovery, and a durable business record without exposing request or response bodies through the recovery log.

## Decision

Proxy attempt lifecycle data is owned by a bounded in-memory `RuntimeAttemptStore` while a request is running. Updates are keyed by `(invoke_id, occurred_at, attempt_public_id)` and replace the previous snapshot for that attempt. The store admits at most 10,000 records and 32 MiB including a 64 KiB terminal reservation per unfinished attempt. Unmaterialized accepted attempts cannot be evicted to admit another request. Terminal diagnostics are bounded at 8 KiB per field; rejected updates increment health counters. The terminal reservation preserves final status, timings and byte counts when optional metadata grows beyond admission capacity.

The terminal journal is the durable admission boundary for begin and terminal invocation records and their final attempt snapshots. Both event types use a versioned JSON entry, sequence and checksum and wait for the 20 ms group-sync window before admission is acknowledged. No request is forwarded before its begin event is confirmed. Confirmed terminals are consumed by the 20 ms / 32 event / 4 MiB SQLite writer batch. A synced begin with no terminal event is recovered as interrupted before normal replay. Admission reserves one maximum-size terminal frame per unmatched begin, so accepting a new begin cannot exhaust the space needed to finish previously accepted requests. Capacity and incomplete append failures are explicit; incomplete writes poison further acknowledgement until restart recovery. If startup cannot durably reconstruct an interruption, it fails closed and preserves the original journal.

SQLite remains the durable query source. The materializer writes the invocation terminal record and attempt snapshots in the same short transaction. Attempt snapshots use the existing `attempt_public_id` unique index and an upsert, so journal replay after a transaction failure is idempotent and does not create duplicate attempt rows.

The terminal transaction also appends the compact Summary source-change descriptor to its existing durable source journal. This descriptor is a recovery obligation, not Summary computation: a fact commit without its descriptor would lose the ability to reconstruct the derived view after restart. Summary computation, statistics, cache refresh and maintenance remain outside the terminal transaction, with independently bounded, retryable consumers.

Routing decisions own a second bounded runtime projection for accepted, not-yet-materialized sticky bindings, account health/cooldown and capability observations. A missing SQLite attempt ID does not mean that a routing outcome can be skipped. The next retry and subsequent request must observe an accepted routing change immediately, without acquiring SQLite write admission. Routing observations enter the recoverable journal before publication to this projection and are materialized asynchronously. Existing failure classifiers and observation ordering remain the source of routing policy.

The routing projection overlays durable SQLite facts and retains unmaterialized changes until commit acknowledgement; normal cache eviction must not discard them. It is bounded at 4,096 events and 16 MiB of decoded events, with a 64 MiB encrypted sidecar and a 128 KiB encoded frame limit. Retained encoded frames and decoded projection memory are accounted as distinct bounded allocations. AES-GCM authenticates version, sequence and event/checkpoint identity using the persisted installation seed; it conceals conversation identifiers and stores canonical reason codes rather than provider error text. Materialization uses the same 20 ms / 32 event / 4 MiB limits and the terminal write class. A random durable event identity deduplicates audit rows; before/after comparisons, account control generations and scoped sticky generations fence replay and delayed results. Projection readers capture pending facts before the SQLite snapshot to prevent an acknowledgement race from losing a committed change.

Account-local, inherited group and global control updates advance `runtime_routing_generation`; credential replacement, disable/enable, routing policy edits and explicit manual recovery invalidate older observations even when values return to their prior state. Informational edits do not invalidate routing observations. Sticky deletion additionally compares the current owner and distinguishes exact model bindings from the fallback binding. Workers belong to the runtime, continue accepting grouped outcomes during HTTP drain, then close admission, sync remaining events and join; unmaterialized confirmed events remain in the sidecar for restart. Capacity rejection can recover after backlog drains; incomplete append or sync failure requires explicit recovery.

Runtime reads use the in-memory projection while a request or unmaterialized routing observation is active and converge to the SQLite record after materialization. Recovery formats must read existing begin/terminal records, provide idempotent acknowledgement and preserve valid synced events following a damaged record; unsupported versions must fail closed without destructive truncation.

## Constraints

- HTTP and SSE response shapes, route selection, retry, and failover behavior remain unchanged.
- Recovery entries contain metadata only; raw request bodies, response bodies, credentials, and unsanitized secrets are excluded.
- Normal proxy attempt lifecycle code does not acquire `InteractiveProxy` write admission or enqueue `AttemptProgress` SQL writes.
- Existing database recovery and orphan repair paths remain available for legacy rows and restart repair.

## Consequences

The request path trades per-attempt SQLite write latency for bounded memory updates and terminal journal durability latency. Terminal persistence can still be delayed by SQLite contention, but that delay is visible as journal and materializer backlog rather than being paid by every progress update. The runtime health response exposes the attempt-store bounds and rejection counters together with terminal-journal backlog and overflow state.

The terminal journal carries a versioned begin event for each attempt and the complete final snapshot set with the terminal invocation event. On restart, a begin event with no matching terminal event is converted into an interrupted terminal record before normal replay, so an attempt that stopped before terminal capture is still materialized once.

## Validation

- Runtime store latest-wins and hard-size rejection are covered by lightweight unit tests.
- Terminal begin/terminal ordering, restart replay, shutdown recovery snapshots, and sensitive-value redaction are covered by terminal-journal tests.
- Terminal attempt materialization is covered by a stateful SQLite idempotence test.
- Validation evidence is recorded against the candidate head and must include Review Policy, lightweight, stateful SQLite (both shards and aggregate), representative scale, archive/file I/O, tooling, and web checks. Local targeted validation covers the runtime-store fence test, terminal journal suite, terminal materialization idempotence test, proxy pressure test, format check, all-target check, and clippy. The pressure scenario observes 32 concurrent attempts while `BEGIN IMMEDIATE` is held and verifies zero `AttemptProgress` SQL enqueue delta and zero additional `InteractiveProxy` admission for the attempt lifecycle.

## Persistent State Compatibility

The candidate reads the existing terminal JSONL layout and existing SQLite rows without rewriting their identities. Additive nullable audit identities and a default-zero account generation reuse existing tables. The encrypted routing sidecar is a new recovery format that older releases do not consume. The persistent-state impact is therefore minor while HTTP/SSE compatibility remains patch. Before rolling back the program, drain confirmed routing facts or retain the sidecar for a forward repair; a rollback does not reverse schema changes or silently discard pending facts. Detailed migration and version-impact records accompany the runtime data-plane Spec.
