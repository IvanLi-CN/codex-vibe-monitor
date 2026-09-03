# ADR 0009: Durable Summary source-change and archive-snapshot recovery

## Status

Accepted

## Context

ADR 0008 makes ordinary terminal tails cheap only while one process retains a
continuous in-memory journal. A restart or a durable change outside that
journal can change the Projection Generation Fence without identifying the
affected selection. The existing fallback then re-admits complete live source
under the four-second Rolling deadline, which can repeatedly fail while an
otherwise exact recent Projection is available.

Separately, a missing or unreadable raw invocation archive can leave a finite
boundary range unavailable. Hourly rollups can prove full compact hours but do
not retain enough information for arbitrary range boundaries, account scope,
or usage/model detail. Returning a successful response in that state would be
an undercount or a partial response.

## Decision

- Add one globally ordered, durable Summary Source Change Journal. Every
  transaction that changes a Summary Projection Generation Fence appends a
  compact Summary Change Descriptor, or a bounded compaction proof, in that
  same transaction. The source change does not commit without one of those
  durable records.
- A descriptor contains only source identity/version, affected account,
  UTC-range, current-rank boundary, and bounded reconstruction keys. It does
  not duplicate raw source text, preview values, or complete Summary rows.
  Existing in-process terminal deltas remain the fast path; after restart,
  RollingDelta reconstructs descriptor keys in bounded source pages.
- Projection and recovery checkpoints retain one Source Change Cursor. The
  active journal tail is bounded to 10,000 entries or 64 MiB. Compacting a
  contiguous tail produces a durable union proof instead of deleting it by
  time. A selection that intersects an unabsorbed proof is unavailable; a
  disjoint exact selection remains available. A broad proof is retained when
  the proof budget is exhausted.
- A RollingDelta must consume a continuous in-memory or durable descriptor
  tail. It may perform bounded key reconstruction off-request, but never
  complete live admission, archive raw hydration, or request-time I/O. A
  missing descriptor becomes a finite or broad fail-closed proof followed by
  bounded reconciliation, not a four-second full Rolling rebuild.
- Persist an immutable, SHA-identified Summary Archive Snapshot in the main
  SQLite database for each completed invocation archive. Snapshot pages are
  compressed, bounded, omit raw text, and retain every Summary field needed to
  reconstruct exact selections. HTTP and SSE never read them directly.
- Make Snapshot coverage an archive cleanup gate. Archive source cleanup is
  permitted only after the matching Snapshot, coverage, and identity proof are
  durably committed. Pressure, capacity, or write failure leaves the
  authoritative archive intact and records recoverable backfill work.
- Run Legacy Summary Snapshot Backfill as a low-priority, seek-paged durable
  checkpoint during ordinary maintenance. It creates snapshots only from
  readable authoritative legacy sources. A permanently missing or unreadable
  legacy source remains a finite range-local unavailable proof until an exact
  source is restored; no approximation is published.
- All-time recovery remains owned by ADR 0005's durable checkpoint. The new
  journal and Snapshot recovery may improve rolling availability but never make
  `all` exact-ready without complete all-time proof.

## Considered Options

- Keep only the in-memory terminal journal. Rejected because restart and
  rollup/archive changes have no continuous durable recovery evidence.
- Maintain separate journals for each source. Rejected because cross-source
  ordering and restart recovery reintroduce unrepresented-change gaps.
- Persist complete Summary contributions or raw archive content in the
  journal. Rejected because it duplicates high-volume data and increases SQLite
  write pressure on normal terminal traffic.
- Store Snapshot sidecars next to raw archives. Rejected because it creates a
  second file-availability dependency identical to the failure being repaired.
- Treat compact hourly rollups as an arbitrary-boundary replacement. Rejected
  because account, usage/model, and partial-hour detail would become inexact.

## Consequences

- Normal terminal writes add no transaction or connection: their compact
  descriptor is batched into the existing source transaction. Journal payload,
  WAL growth, commit latency, lock retries, compaction work, and bounded
  reconstruction latency require telemetry and representative-scale gates.
- Main SQLite gains bounded active-journal/proof state and growing compact
  archive Snapshot state. Snapshot page and transaction caps, storage telemetry,
  and production-shaped size validation are required before release.
- Recent exact selections no longer depend on a process-local journal surviving
  restart or on a full live re-admission finishing within the Rolling deadline.
- The system cannot reconstruct a legacy range for which every authoritative
  source is gone. That range remains selection-local unavailable rather than
  returning a fabricated successful response.
