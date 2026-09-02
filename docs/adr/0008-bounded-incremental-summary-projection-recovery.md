# ADR 0008: Bounded incremental Summary projection recovery

## Status

Accepted

## Context

An already published Summary Projection can become unavailable when a normal
Rolling refresh re-admits the complete live source and exceeds its bounded
deadline. Terminal writes already provide an ordered, SQLite-acknowledged
in-memory delivery path, but treating every such change as a reason to rebuild
the entire Projection couples recent exact availability to historical source
admission.

## Decision

- Retain a Summary-owned, bounded `Summary Delta Journal`. A pending
  registration establishes its monotonic `SummaryDeltaCursor` before enqueue,
  but only the matching SQLite commit acknowledgement promotes the compact
  terminal values needed to reduce a published rolling base. Rejected enqueue
  rolls the pending registration back.
- `RollingDelta` renews a published rolling Projection from a continuous,
  non-overflowed journal without invoking complete live admission or archive
  hydration. A missing or unknown durable change remains on the existing
  fail-closed Rolling recovery path.
- Journal capacity is 10,000 entries or 64 MiB. Capacity overflow records a
  bounded `DeltaGapProof` with time and account scope. A request intersecting
  that proof is unavailable; a disjoint rolling selection remains exact. A
  missing sequence has no durable range proof, so it remains broad unavailable
  until reconciliation instead of borrowing a later entry's scope.
- HTTP and Summary SSE compose a Projection and its acknowledged journal from a
  single hub-state snapshot. They remain memory-only and do not query SQLite,
  archives, or files. Ordered `current` selections without a complete rank proof
  are unavailable rather than additively patched.
- All-time coverage remains owned by the durable all-time checkpoint from ADR
  0005. A RollingDelta never makes `all` ready.

## Considered Options

- Extend the Rolling deadline. Rejected because it preserves a global stale
  failure mode and merely delays source-pressure recovery.
- Serve the last Projection after an unknown delta. Rejected because it labels
  known-stale data as exact.
- Add request-time SQLite or archive reads. Rejected because recovery work must
  remain outside Summary HTTP and SSE paths.

## Consequences

- Normal terminal traffic no longer requires a full live-source read before
  recent Summary windows can remain exact-ready.
- Capacity or ordering faults fail closed only for affected selections and are
  recoverable by the existing generation-fenced reconciliation path.
- The service retains one bounded compact delta queue in addition to the
  immutable Projection; all-time correctness and publication remain independent.
