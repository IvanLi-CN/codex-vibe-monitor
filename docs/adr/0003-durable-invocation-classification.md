# ADR 0003: Durable invocation classification

## Status

Accepted

## Context

Summary Projection must remain an exact memory-only read model even when durable
history contains large raw payloads. The existing `failure_class` field is not a
complete compatibility contract for historical records: some legacy rows require
payload or raw-response diagnostics to resolve their outcome. If Summary avoids
those bytes while full statistics and rollups continue to inspect them, one
invocation can be classified differently by the selected window or materialized
source. Retaining every payload in the projection fixes neither memory pressure
nor the split-brain classification.

## Decision

- Treat the terminal invocation classification (`failure_class`, actionable state,
  and its classification revision) as a durable, versioned fact. A terminal
  writer computes and commits it atomically with the terminal record.
- Materialize legacy and immutable-archive classifications through a bounded,
  resumable, pressure-aware background path. Immutable archives use a durable
  identity-keyed overlay and coverage proof rather than archive rewrites.
- Summary Projection, hourly rollups, and all aggregate readers consume only the
  canonical classification fact. They do not re-derive outcomes from payload or
  raw response bytes.
- A range with incomplete canonical classification coverage is exact-unavailable;
  it is never silently treated as success, reconstructed in an HTTP request, or
  represented by partial totals. The background materializer may repair that
  coverage and publish a later exact snapshot.

## Considered Options

- Keep raw payload diagnostics in every Summary record. Rejected because large
  payloads reintroduce unbounded memory admission and do not unify rollups.
- Let each reader retain its own legacy fallback. Rejected because result
  semantics then depend on the reader and materialization state.
- Rewrite historical archive files in place. Rejected because archive identities
  are immutable and a durable primary-store overlay is recoverable and bounded.

## Consequences

- Classification changes are an expand-and-backfill migration with explicit
  coverage state, not a read-time compatibility heuristic.
- Rollup publication must only claim exact coverage after its source records have
  current canonical classification, so legacy repair can safely recompute affected
  buckets.
- The materializer is low-priority work: it yields on pressure, cancellation and
  its bounded transaction budget, while terminal persistence remains P1.
