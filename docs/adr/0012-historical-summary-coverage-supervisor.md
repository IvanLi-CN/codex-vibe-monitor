# ADR 0012: Historical Summary Coverage Recovery Supervisor

Status: Accepted

## Context

Summary's recent selections must remain exact and memory-only while historical
archive coverage converges under a bounded maintenance budget. A single refresh
owner previously mixed rolling publication with all-time manifest proof, raw
archive hydration, and legacy Snapshot recovery. A slow or missing archive could
therefore repeat broad work, delay publication, or make an unrelated recent
selection unavailable.

The repository already has durable all-time and Snapshot V2 cursors. The missing
boundary is runtime ownership: those cursors need one scheduler that can advance
one verified page, yield to pressure and restart from committed progress without
calling the generic Summary Projection builder.

## Decision

Introduce `SummaryCoverageRecoverySupervisor` as the only maintenance owner for
historical Summary recovery. It coordinates two bounded workers:

- All-time raw replacement and manifest/rollup proof advances use the existing
  generation-fenced all-time checkpoint. Each verified page commits its cursor,
  accumulator, and proof before the next page is considered.
- Legacy archive recovery uses the existing seek-paged Snapshot V2 backfill. It
  prioritizes candidates intersecting the current 30-day horizon, verifies
  manifest SHA, row count, coverage, page order, and semantic decoding, and only
  then records V2 authority. Missing or invalid sources become a finite
  unavailable outcome; a pressure or SQLite lock defer leaves the uncommitted
  cursor unchanged.

The regular Summary refresh path publishes and renews the exact rolling
Projection independently. It never calls the generic all-time build for an
unfinished checkpoint and never opens paged raw archive sources. Once a
checkpoint scope is exact-ready, the supervisor atomically merges its totals
and the bounded live-tail overlay into the published Projection. A changed
coverage fence retries the bounded supervisor pass; it does not revoke an
already exact recent Projection.

HTTP and Summary SSE continue to read only the immutable hub Projection and its
availability overlay. A selection intersecting an unproven temporal, account,
or current-rank boundary remains `unavailable`; disjoint exact selections stay
available. The supervisor emits only stage, scope, count, cursor, proof, and
duration telemetry and never places source payload in logs or responses.

Legacy Snapshot V2 recovery uses a versioned `(UTC occurred_at, id)` seek key,
matching the proof validator's total order. Each verified page, its next seek
key, and its manifest outcome are committed atomically. Retryable source or
SQLite conditions use bounded backoff; semantic, timestamp, row-count, schema,
or manifest-identity failures are quarantined for that manifest identity and
are reconsidered only when its SHA changes. An old ID-only cursor is never
combined with a V2 proof; unproven pages are discarded and rebuilt in the V2
order.

## Alternatives considered

- Keep all-time and Snapshot recovery inside the generic Projection builder:
  rejected because one slow archive can block or repeatedly rebuild unrelated
  recent selections.
- Run Snapshot backfill only from startup rollup maintenance: rejected because
  startup ownership couples low-priority archive I/O to rollup scheduling and
  does not provide one recovery owner or 30-day prioritization.
- Reconstruct historical totals in the HTTP handler: rejected because it breaks
  the Projection-only and zero request-time I/O contract.

## Consequences

Historical recovery has one observable owner, bounded page commits, and durable
restart progress. Recent Summary availability no longer depends on all-time
archive latency. The supervisor adds maintenance scheduling and a finalization
merge, and all-time exactness may remain unavailable until its proof pages and
Snapshot V2 authority complete. This is intentional fail-closed behavior: no
partial or approximate totals are published.
