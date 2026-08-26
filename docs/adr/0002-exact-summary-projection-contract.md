# ADR 0002: Exact Summary projection contract

## Status

Accepted

## Context

Summary is a high-frequency operational read surface, while its durable facts span
live invocations, archive materialization and rollups. Rebuilding it in the HTTP
handler shifts SQLite and file I/O into the hot path. Returning a partial rollup,
an empty placeholder or an unbounded in-memory history instead preserves neither
the response contract nor production performance.

Low-priority recovery also shares SQLite with P1 durability. A gate refusal before
database access is different from a real SQLite lock after access begins; treating
them as one retry class causes unnecessary polling and audit traffic.

## Decision

- Treat Summary Projection as an exact, immutable in-memory read model. A valid
  Summary HTTP request reads only that model after query validation.
- Build exact snapshots outside the request path from durable rollups for fully
  covered interiors plus exact records for live tails, partial boundaries,
  account lag and coverage gaps.
- Keep canonical Summary records compact: retain only normalized fields consumed
  by `StatsResponse`; raw invocation payloads, raw responses and unrelated large
  text remain durable source data and do not consume projection admission.
- Preserve the existing wire shape. A missing initial exact snapshot keeps the
  existing unavailable behavior; partial or fabricated zero data is never a
  normal response.
- Retain last-good data only as a diagnosed background-refresh state, never as a
  falsely fresh projection.
- Model pressure-gate refusal as `SQLite Pressure Defer` with one due/event wake.
  Model actual `BUSY`/`LOCKED` results as separate bounded operation failures.
- Treat hourly rollup and P2 batch finalization as low-priority writers that
  yield under pressure. Their bounded deferred backlog must never take priority
  over terminal P1 durability.
- Keep long-term migration cursor-driven, cancellable and capped to 512-row
  write transactions so P1 terminal work remains dominant.

## Consequences

- Summary recovery requires explicit coverage accounting and testable exactness
  across rollup, archive and account scopes.
- A raw-byte cap is not a valid reason to leave ordinary Summary history
  permanently unavailable. Capacity control applies to the compact canonical
  representation and coverage set, not to unconsumed source payload bytes.
- Background work must carry future eligibility and wake evidence instead of
  using short retry tickers or no-op task records; repeated notifications during
  one active cooldown do not redispatch the same task.
- Some first-load requests remain unavailable until an exact snapshot exists;
  this is preferable to silently serving incorrect statistics.
- Release publication does not authorize deployment. Production evidence is
  collected only through owner-confirmed, read-only observation.
