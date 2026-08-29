# ADR 0004: Summary archive publication proof

## Status

Accepted

## Context

A `completed` invocation archive is consumed by Summary's durable rollup path.
Writing that status before its SHA-bound rollup proof makes a correct archive
indistinguishable from an incomplete Summary source, and a background repair
that only notices one optional marker can leave the state permanent. Code-only
ordering is insufficient because a future writer can bypass it.

## Decision

- Treat `completed` `codex_invocations` archives as Summary-eligible only when
  finite coverage, the current manifest SHA, historical materialization, and
  all required Summary rollup proofs are present.
- Publish new invocation archives through an internal materializing state, then
  use a database finalization constraint to permit `completed` only after that
  proof exists in the same transaction as the source deletion.
- A normal application update automatically reconciles legacy completed
  archives in the bounded historical-rollup scheduler through source-identity
  and full bucket-closure verification or rebuild. It never promotes a
  materialized timestamp or a hand-written marker into proof, and it requires
  no operator command.

## Considered Options

- Code-only retention helper: rejected because it cannot prevent a future
  publication path from reintroducing an incomplete completed archive.
- Marker-only legacy patch: rejected because it can certify duplicate, stale,
  missing, or otherwise unverified compact rollups.

## Consequences

The normal release path owns both schema protection and legacy recovery. P1
durability remains ahead of the bounded, pressure-aware reconciliation work,
while Summary HTTP and SSE remain memory-only readers.
