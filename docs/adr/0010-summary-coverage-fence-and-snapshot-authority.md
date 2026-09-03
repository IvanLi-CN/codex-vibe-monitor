# ADR 0010: Summary coverage fence and Snapshot authority

## Status

Accepted

## Context

Summary has two independent recovery dimensions. The live terminal tail changes
frequently and must remain recoverable without restarting historical coverage;
archive manifest and rollup changes define a slower historical generation. A
single generation comparison made every committed terminal invalidate the
all-time checkpoint and could send the next refresh back through bounded live or
raw archive admission.

Archive snapshots also have two compatibility generations. Existing V1 pages
are useful evidence that backfill work exists, but their payload does not prove
that every Summary field can be decoded. Treating them as cleanup authority can
remove the only exact source for a boundary range.

## Decision

- Represent the historical generation with a `SummaryCoverageFence` containing
  the completed manifest high-watermark. A terminal or rollup tail update does
  not invalidate this fence or reset a completed all-time checkpoint.
- Represent post-fence progress with a `SummaryLiveTailCursor` containing live,
  rollup and terminal sequence cursors. `RollingDelta` may reconstruct this
  bounded tail, but it never falls back to complete live admission or raw
  archive hydration.
- Bootstrap and ordinary rolling refreshes consume live rows, compact proof and
  manifest metadata only. A boundary that needs raw replacement is recorded as
  a range-local unavailable proof until the independent AllTime reconciler or a
  verified Snapshot proves it.
- Only a semantically decoded, SHA- and coverage-verified
  `SummaryArchiveSnapshotV2` is a cleanup authority. V1 remains backfill input
  only. Snapshot recovery is off-request and an unavailable or corrupt Snapshot
  leaves the authoritative archive intact.

## Considered Options

- Keep one fence for live and historical sources. Rejected because ordinary
  terminal traffic repeatedly invalidates unrelated all-time progress.
- Let Bootstrap synchronously repair archive boundaries. Rejected because a
  large archive can prevent the first exact Projection from publishing.
- Accept V1 pages based only on their stored hash. Rejected because a hash does
  not prove page semantics, coverage, or row-count integrity.

## Consequences

- Recent Summary selections can remain Exact-Ready while all-time coverage
  advances or waits independently.
- Boundary gaps are visible and fail closed only for intersecting selections;
  no partial aggregate is published.
- Main SQLite stores a small amount of fence/checkpoint metadata and verified
  normalized Snapshot data. Raw text and preview payloads remain outside the
  Snapshot format, and HTTP/SSE continues to read only the published in-memory
  Projection.
