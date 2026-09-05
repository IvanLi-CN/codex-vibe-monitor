# ADR 0014: Durable Summary coverage recovery

Status: Accepted

## Context

Historical Summary coverage must converge without an `all` HTTP request or an
SSE owner. The previous maintenance path skipped the supervisor without an
all-time owner, and its Snapshot V2 selector combined 30-day priority with a
monotonic archive-ID cursor. A busy newer tail could permanently postpone an
older archive that intersected the current 30-day window. A deadline before
raw-source hashing also restarted the entire SHA-256 scan on every attempt.

Those failures leave exact current and short rolling selections available but
can keep 30-day and all-time coverage unavailable indefinitely. Retrying in the
handler, increasing deadlines, or returning a partial aggregate would violate
the exact Projection contract.

## Decision

- `SummaryCoverageRecoverySupervisor` is an off-request maintenance owner. It
  receives a bounded cadence turn even when there are no Summary HTTP/SSE
  owners, while Bootstrap and Rolling remain demand-driven.
- The supervisor obtains one low-priority global database-pressure permit before
  checkpoint, manifest, or archive access. A denied permit performs no durable
  progress I/O. All-time checkpoint work and V2 backfill each receive an
  independent bounded turn; a ready all-time checkpoint does not skip V2
  authority recovery.
- Snapshot V2 backfill evaluates a durable due queue before its archive-ID
  fairness sweep. The due queue prioritizes the current 30-day horizon and due
  retry outcomes. Complete and quarantined manifest identities are excluded
  until their manifest SHA changes. A deadline persists `Deferred/budget` with
  the prior cursor instead of silently dropping the attempt.
- SHA-256 proof is resumable. The outcome stores the algorithm/state version,
  consumed byte offset, serialized standard-hash state, final verified digest,
  and filesystem source fingerprint. A changed fingerprint discards unproven
  pages and hash state; EOF must match the manifest SHA before V2 pages can be
  committed. Verified V2 page, semantic proof, seek cursor, outcome, and
  coverage metadata remain short-transaction commits.
- V1 Snapshot pages are never cleanup proof. Raw archive authority remains until
  the matching V2 proof has been fully verified. HTTP and SSE only read the
  immutable Projection and availability overlay; unproven intersections remain
  unavailable rather than partial or stale success.
- A verified V2 proof advances the historical coverage fence and immediately
  triggers one bounded metadata-only RollingDelta publication, independent of
  request ownership. Ordinary hot/live rollup writes advance only the live-tail
  cursor; archive replay proof advances the historical fence. A ready checkpoint
  whose coverage is already published is not finalized again. Global and account
  all-time aggregates retain their own published coverage fences, so a RollingDelta
  generation update cannot make a retained aggregate appear current.
- Retryable recent candidates obey their persisted `next_probe_at` eligibility.
  An idle completed backfill checkpoint performs no repeated progress write.

## Alternatives considered

- Trigger recovery only after `all` interest: rejected because it makes durable
  coverage dependent on a client request and leaves unattended 30-day gaps.
- Use the archive-ID cursor as both priority and progress: rejected because new
  IDs can starve an older, still-relevant archive.
- Rehash from byte zero after every deadline: rejected because a large archive
  can consume every bounded maintenance budget without producing progress.
- Trust a V1 payload hash for cleanup: rejected because it does not prove V2
  semantic fields, page order, coverage, or manifest identity.

## Consequences

Recovery makes bounded, observable forward progress without changing the HTTP
or SSE wire shape. It adds small outcome metadata and a second SHA-256 crate
alias solely for serializable recovery state; existing HMAC uses remain on
`sha2` 0.10. The system remains fail-closed: if a source is missing, changed,
or cannot be proved within a bounded attempt, only the affected selection stays
unavailable until an exact authority exists. Continuous live rollup traffic no
longer restarts historical checkpoint pages, and completed recovery no longer
repeats all-time usage and Snapshot finalization work on every idle cadence.
