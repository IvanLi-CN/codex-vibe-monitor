# ADR 0034: Summary Delta degraded read path

## Status

Accepted

## Context

The Summary Delta Journal can retain a bounded proof gap after a dropped, conflicting, or out-of-order terminal update. The existing request path treated every selection intersecting that in-memory gap as unavailable, even when an immutable last-good Projection and acknowledged terminal overlay were already published. That made `/api/stats` and `/api/stats/summary` fail during a recoverable background proof window and encouraged callers to retry a path that must not perform archive or full-history I/O.

## Decision

- When a selection intersects only a pending Summary Delta Journal proof, serve the published last-good immutable Projection plus the acknowledged in-memory terminal overlay.
- Mark the response with the additive `dataQuality` object `{ "state": "degraded", "proofPending": true, "reason": "summary_delta_journal_pending" }`. Healthy responses omit this field.
- Route legacy `/api/stats` through the all-time Summary memory handler so it has the same no-request-I/O and degraded-read behavior.
- Bypass the normal freshness rejection only for this explicit proof-pending case. No published Projection, archive/source coverage gap, current-rank uncertainty, or other durable proof failure remains eligible for this response and continues to return `unavailable`.
- Keep proof and reconciliation off-request. Archive marker discovery is resumable and limited to a fixed 128-row metadata batch, with an order-aligned partial index for the candidate scan.

## Alternatives Considered

- Keep returning `503` for every Delta gap. Rejected because a bounded in-memory gap does not invalidate the already published snapshot and turns recoverable maintenance into client-visible downtime.
- Rebuild or prove the gap synchronously. Rejected because it violates the Summary zero-I/O read contract and can scan large live or archive history under request latency.
- Return the last-good data without a marker. Rejected because callers need to distinguish an exact response from one whose missing journal proof is still pending.

## Consequences

The response contract gains one optional data-quality field, while healthy payloads remain unchanged. Consumers can keep rendering the latest known totals and can surface degraded state without treating it as exact. Background reconciliation remains responsible for closing the proof; no archive or database work is moved into HTTP. The last-good snapshot is not presented as fresh or exact while the marker is present.

This decision narrows the fail-closed behavior from ADR 0008, ADR 0012, and ADR 0014 only for an in-memory Summary Delta Journal proof gap. Durable archive, source, classification, and coverage failures retain their existing exact-or-unavailable behavior.
