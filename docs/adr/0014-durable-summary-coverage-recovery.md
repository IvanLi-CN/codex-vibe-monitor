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
can keep 30-day and all-time coverage unavailable indefinitely. A backfill
attempt can also report `complete` after writing pages while no complete V2
proof exists; treating that attempt as coverage makes the next Supervisor pass
silently report zero candidates. Retrying in the handler, increasing deadlines,
or returning a partial aggregate would violate the exact Projection contract.

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
  publishes one generation-fenced immutable Coverage Publication Overlay,
  independent of request ownership. Overlay publication reduces only verified
  normalized contributions and availability state; it never starts a generic
  RollingDelta, live admission, or paged raw hydration. Ordinary hot/live rollup
  writes advance only the live-tail cursor; archive replay proof advances the
  historical fence. A ready checkpoint whose coverage is already published is
  not finalized again. Global and account all-time aggregates retain their own
  published coverage fences, so a live-tail update cannot make a retained
  aggregate appear current.
- Live-tail recovery uses the durable `(row_id, invoke_id, occurred_at)` source
  identity rather than sequence numbers alone. An ACK or restart replay that
  matches the identity already present in the immutable Projection is absorbed
  idempotently; a missing identity remains a broad fail-closed proof. A bounded
  `SummaryLiveTailReconciliationCheckpoint` fixes one target watermark and
  source cursor under the existing pressure permit. Its CAS publication can
  retain later committed entries as an overlay and never invokes generic
  RollingDelta, Bootstrap, full live admission, or archive hydration.
- Live-tail readiness is exposed only as no-payload diagnostics (reason, stage,
  gap count, watermark, epoch, and elapsed time). The release gate requires
  `current`, `1d`, `7d`, and `today` to be exact; `30d` and `all` may be exact or
  an explicitly historical local unavailable result, but a live-tail gap blocks
  the candidate.
- Retryable recent candidates obey their persisted `next_probe_at` eligibility.
  An idle completed backfill checkpoint performs no repeated progress write.
- Snapshot page progress and coverage authority are separate durable states.
  Each completed archive manifest has a `SummaryCoverageObligation`; an attempt
  outcome never satisfies it. The identity-bound V2 final-proof marker is
  committed only after complete page, manifest, ordering, row-count and
  semantic verification. Page or manifest mutation revokes that marker and
  reopens the obligation. Legacy terminal outcomes are migrated to explicit
  range-local gaps, and a changed manifest SHA creates a fresh obligation.

## Alternatives considered

- Trigger recovery only after `all` interest: rejected because it makes durable
  coverage dependent on a client request and leaves unattended 30-day gaps.
- Use the archive-ID cursor as both priority and progress: rejected because new
  IDs can starve an older, still-relevant archive.
- Rehash from byte zero after every deadline: rejected because a large archive
  can consume every bounded maintenance budget without producing progress.
- Trust a V1 payload hash for cleanup: rejected because it does not prove V2
  semantic fields, page order, coverage, or manifest identity.
- Treat a successful attempt outcome as coverage: rejected because an
  interrupted writer can persist `complete` without a complete V2 page set and
  thereby hide the remaining recovery obligation.

## Consequences

Recovery makes bounded, observable forward progress without changing the HTTP
or SSE wire shape. It adds small outcome metadata and a second SHA-256 crate
alias solely for serializable recovery state; existing HMAC uses remain on
`sha2` 0.10. The system remains fail-closed: if a source is missing, changed,
or cannot be proved within a bounded attempt, only the affected selection stays
unavailable until an exact authority exists. Continuous live rollup traffic no
longer restarts historical checkpoint pages, and completed recovery no longer
repeats all-time usage and Snapshot finalization work on every idle cadence.
Intermediate Snapshot pages no longer churn the historical coverage fence, so a
multi-page recovery cannot repeatedly cancel its own AllTime checkpoint. The
Supervisor telemetry distinguishes pending obligations, retryable attempts,
terminal gaps and verified proofs, making a zero-candidate pass explainable.
