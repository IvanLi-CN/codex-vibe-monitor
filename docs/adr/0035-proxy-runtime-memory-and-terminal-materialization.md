# ADR 0035: Proxy runtime memory state and terminal materialization

## Context

The proxy request path previously used the SQLite write coordinator for attempt creation, progress updates, response metadata, and terminal updates. A slow or contended SQLite writer could therefore add database admission latency to an otherwise active upstream request.

The proxy runtime also needs bounded live state, crash recovery, and a durable business record without exposing request or response bodies through the recovery log.

## Decision

Proxy attempt lifecycle data is owned by a bounded in-memory `RuntimeAttemptStore` while a request is running. Updates are keyed by `(invoke_id, occurred_at, attempt_public_id)` and replace the previous snapshot for that attempt. The store evicts expired and oldest entries at fixed record and byte limits; an oversized snapshot is rejected and reported through health state.

The terminal journal is the durable admission boundary for terminal invocation records and their final attempt snapshots. A terminal event is written with a versioned JSON entry, sequence and checksum, force-synced before admission is acknowledged, and then consumed by the existing 20 ms / 32 event / 4 MiB SQLite writer batch. Attempt begin events use the 20 ms journal group-commit window; the begin call reports journal append acceptance, while a hard process loss before the next group sync may lose only that unacknowledged begin event. A begin event that reaches the journal is replayed after close/reopen, and a force-synced begin event is recovered as an interrupted terminal when no terminal event exists. If the journal cannot accept an event, the enqueue result is failure and the caller receives an observable error path; a terminal event is never reported as successfully accepted from unbounded memory alone.

SQLite remains the durable query source. The materializer writes the invocation terminal record and attempt snapshots in the same short transaction. Attempt snapshots use the existing `attempt_public_id` unique index and an upsert, so journal replay after a transaction failure is idempotent and does not create duplicate attempt rows.

Derived projections and maintenance work remain outside the terminal transaction. Runtime reads use the in-memory projection while the request is active and converge to the SQLite record after terminal materialization.

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
- Validation evidence for the current branch head: the required CI run passed Review Policy, lightweight, stateful SQLite (both shards and aggregate), representative scale, archive/file I/O, tooling, and web checks. Local targeted validation also passes the runtime-store fence test, terminal journal suite, terminal materialization idempotence test, proxy pressure test, format check, all-target check, and clippy. The pressure scenario observes 16 concurrent attempts while `BEGIN IMMEDIATE` is held and verifies zero `AttemptProgress` SQL enqueue delta and zero additional `InteractiveProxy` admission for the attempt lifecycle.
