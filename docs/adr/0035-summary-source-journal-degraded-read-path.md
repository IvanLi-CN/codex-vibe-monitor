# ADR 0035: Summary Source Journal degraded read path

## Status

Accepted

## Supersedes

[ADR 0034: Summary Delta degraded read path](0034-summary-delta-degraded-read-path.md)

## Context

The durable Summary Source Change Journal can retain a broad compaction proof while the immutable Summary Projection and an acknowledged in-memory terminal overlay are already available. Treating that proof-only condition as a universal outage makes the HTTP and `SummaryCurrent` read paths return `503` even though they can expose a bounded last-good view without request-time recovery work.

## Decision

- An intersecting broad source-journal compaction proof may use the published immutable Projection plus acknowledged in-memory overlay when the overlay is available; the response must carry the existing `dataQuality` proof-pending marker.
- HTTP Summary, legacy `/api/stats`, initial subscription snapshots, and reconnect snapshots use the same selection-local eligibility and response path. They do not advance a source cursor, clear a proof, or perform SQLite/archive/file I/O.
- Scoped source identity gaps, late historical coverage gaps, missing overlays, invalid overlay timestamps or identities, missing baselines, archive/classification gaps, and terminal current-rank gaps remain unavailable. The response builder's independent durable coverage checks remain authoritative.
- Background source reconciliation and audit continue to close the proof. A degraded response is never treated as fresh or exact, and healthy responses continue to omit `dataQuality`.

## Consequences

The existing additive proof-pending wire marker now covers the recoverable source-journal compaction window as well as the terminal delta proof window. A caller can keep rendering the acknowledged live increment while distinguishing it from a fully proven snapshot; genuinely unsafe source or coverage conditions retain fail-closed behavior.
