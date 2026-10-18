# Bounded Invocation Range Preparation and Demand Priority

## Status

Accepted

## Context

Invocation range allocation already keeps committed ranges in memory, reserves
the next range in SQLite, and publishes it only after commit. The request path
has a fixed 100 ms empty-range budget. Background preparation currently treats a
temporary pool or `P2Derived` admission miss as a reason to remove its
speculative owner immediately. A request can then arrive while a low-priority
preparation operation is still active and wait for work that was not requested
by the caller.

## Decision

Background preparation uses a bounded in-memory queue keyed by owner, cache
generation, and UTC hour. Eligible existing owners are deduplicated and retained
when preparation is deferred by pool availability, SQLite pressure, cache
capacity, or an in-flight operation. Retry timing is bounded and capped; the
queue is never allowed to grow beyond the existing startup preparation limit.

An owner requested by the live allocation path is promoted ahead of speculative
preparation. Demand may reuse a committed current or standby range immediately,
or start the normal interactive reservation path when no committed range is
available. A speculative operation must not make an existing owner wait for a
low-priority attempt past the existing 100 ms request budget. A genuinely new
owner that cannot obtain durable reservation authority retains the existing
bounded failure behavior.

Durable range reservation, sequence ceilings, owner generations, cancellation
drain, and commit-before-memory-publication remain unchanged. No new database
table, ID format, public response field, or per-invocation SQL operation is
introduced. Diagnostics identify bounded queue deferral, demand promotion,
committed publication, and the existing timeout outcome without logging raw
prompt-cache keys.

## Consequences

Existing active owners receive a bounded opportunity to have a committed range
ready before the next request. Temporary database pressure no longer erases the
only preparation signal, while request demand is not held behind speculative
work. A deferred owner remains recoverable in memory until it is retried,
evicted by the existing lifecycle, or shutdown drains the coordinator.

The 100 ms request boundary remains an explicit availability boundary rather
than being hidden by a larger timeout. Failed or cancelled reservations remain
unpublished and their durable ceilings remain authoritative.

## References

- [Proxy Invocation Identity](../specs/proxy-invocation-identity/SPEC.md)
- `src/prompt_cache_conversations/invocation_ranges.rs`
- `src/prompt_cache_conversations/invocation_ranges/preparation.rs`
