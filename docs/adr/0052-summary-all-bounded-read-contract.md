# Summary All bounded aggregate and paged-read contract

Status: Proposed

## Context

The operational Summary All surface is `GET /api/stats/summary?window=all`, with an
optional `upstreamAccountId`. It returns the existing `StatsResponse` wire shape. It is
not the same contract as `AnalyticsReadStore::overview` or its
`AnalyticsOverview.global` field: the latter is the long-term `7d|30d|180d|365d`
analytics surface and uses a different metric set.

The current operational implementation serves Summary All from an exact in-memory
`SummaryProjection`. Historical archive/rollup coverage, the live tail, global scope,
and account scope are admitted independently. Missing proof, an unhydrated known
account, or an expired last-good projection is unavailable; the handler does not turn
those states into a full-history SQL scan or fabricated zero values.

The PostgreSQL foundation currently provides normalized `analytics_facts`, daily and
hourly long-term rollups, replica state, and projection state. Its daily/hourly rollup
columns contain calls, token/cost/timing metrics and sample counts, but do not contain
the operational Summary All success/failure/non-success-cost and usage-breakdown
contract. Therefore, a single aggregate SQL row over `analytics_facts` would still be
an unbounded read, and the existing long-term rollups are not by themselves an exact
Summary All authority.

This ADR defines the future read contract without changing the public response, the
fact schema, rollup schema, or write semantics.

## Decision

### 1. Keep the surfaces separate

- `SummaryAll` means the operational `StatsResponse` described above.
- `AnalyticsOverview` and `AnalyticsSeriesResponse` remain the long-term analytics
  contract. Their range, dimensions, daily points, and sample semantics are governed
  by `docs/specs/5k89c-long-term-usage-analytics/SPEC.md` and
  `docs/specs/pg18-analytics/SPEC.md`.
- A future PostgreSQL implementation MUST NOT silently reinterpret the long-term
  `global` aggregate as the operational Summary All response.
- This decision does not add a public page cursor or change `StatsResponse`.

### 2. Admission is a fenced snapshot

Every read that contributes to one Summary All response MUST first admit a single
read context containing:

- a service-eligible PostgreSQL replica with current fact-date semantics, a complete
  baseline, and matching source/target checkpoints;
- `analytics_state.status = 'ready'`;
- the replica generation, coverage generation/revision, and coverage watermark;
- the immutable historical aggregate generation and its exclusive live-tail cursor;
- an explicit live-tail high watermark captured before the first page; and
- the requested scope: global or one positive upstream account ID.

For a one-shot response, admission and every fact/rollup query MUST run in the same
`REPEATABLE READ, READ ONLY` PostgreSQL transaction. The implementation MUST NOT
re-evaluate “now”, the replica checkpoint, or the live high watermark for each page.

A maintenance or recovery reader that needs to resume across transactions MUST carry
the full opaque cursor and the aggregate/fence identity. That cursor is restartable
state, not a portable PostgreSQL snapshot. The next transaction MUST revalidate the
same immutable aggregate generation and coverage fence; a changed generation,
coverage revision, checkpoint, or source acknowledgement invalidates the cursor and
returns unavailable/restart-required rather than mixing snapshots.

### 3. Compose a bounded exact aggregate

An exact Summary All response has two disjoint contributions:

1. A verified historical prefix from a compact aggregate authority. The authority
   MUST prove complete coverage through an exclusive key and MUST contain every field
   required by the operational response: total count, success count, failure count,
   total tokens, total cost, non-success cost, and any usage-breakdown fields that
   the selected response includes. Its archive manifests, source generation, and
   materialization identity MUST be verified before admission.
2. A bounded live tail after that exclusive key and through the captured high
   watermark. The live tail is read with keyset pages and merged exactly once into
   the historical prefix.

The current PostgreSQL daily/hourly long-term rollups cannot satisfy item 1 for the
operational response. Until a separately reviewed compact authority exists, an
explicitly selected PostgreSQL Summary All path MUST remain unavailable; it MUST NOT
scan all facts and MUST NOT fall back to an expensive SQLite request after PostgreSQL
was selected. Keeping the existing SQLite Summary All path is allowed while this
authority is absent.

The merge MUST preserve the existing Summary projection classification:

- every admitted invocation fact contributes one to `totalCount` and its normalized
  token/cost values;
- a success requires the persisted fact classification to be equivalent to the
  existing clean `success`/`completed`/warning-success or clean `http_200` rule;
- a terminal fact with a non-empty, non-`none` resolved failure class contributes to
  `failureCount` and `nonSuccessCost`;
- a live `running` or `pending` fact contributes to totals but to neither success nor
  failure; archived pending rows are terminal only when the historical authority has
  explicitly applied the existing archived-row rule; and
- missing/invalid classification, non-finite numeric values, or arithmetic overflow
  invalidates the read. It MUST NOT be silently converted into success or zero.

The fact contract must therefore carry a resolved classification equivalent to the
operational resolver. `success_like` derived only from the current PG snapshot helper
is not sufficient to prove parity for warning-success or clean `http_200` rows.

### 4. Keyset page contract

The live-tail page order is the non-null tuple:

```text
(stats_date, occurred_at_epoch_ms, source, source_row_id)
```

Pages MUST use strict tuple continuation and the matching order, equivalent to:

```sql
WHERE (stats_date, occurred_at_epoch_ms, source, source_row_id)
      > (:stats_date, :occurred_at_epoch_ms, :source, :source_row_id)
ORDER BY stats_date, occurred_at_epoch_ms, source, source_row_id
LIMIT :page_rows
```

The cursor MUST also bind the aggregate generation, coverage revision, source
checkpoint, scope, and high watermark. `OFFSET`, an order by timestamp alone, a
mutable “latest now” cutoff, and a cursor containing only `source_row_id` are not
valid. `(source, source_row_id)` is the current fact identity and prevents a
cross-source tie from being treated as the same row.

The initial implementation envelope is:

- at most 256 facts and at most 1 MiB of encoded fact data per page;
- at most 200,000 live-tail rows and 32 MiB of materialized compact aggregate data
  for one read; and
- an explicit request/deadline budget for the page loop.

These values reuse current bounded ingestion and Summary projection precedents. They
are safety ceilings, not a throughput or p95 claim. A single row that exceeds the
encoded page budget, a tail that exceeds the total budget, or a page that cannot be
validated within the deadline is unavailable. Returning the prefix plus a partial
tail is forbidden.

For a public one-shot response, the pages are internal to one transaction and the
endpoint still returns one `StatsResponse`. A future maintenance/recovery API may
return an opaque continuation cursor, but exposing that cursor through the current
public Summary response requires a separate response-contract decision.

### 5. Coverage, freshness, and scope isolation

Global and account coverage are independent:

| Scope   | Required proof                                                                                        | Failure effect                                                                         |
| ------- | ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Global  | Complete global historical authority, global live-tail fence, and global source/replica generation    | Global Summary All is unavailable                                                      |
| Account | Complete account historical authority, account live-tail fence, and account source/replica generation | That account Summary All is unavailable; a proven global response may remain available |

The serving freshness budget MUST reuse the existing Summary All budget of 15 seconds
unless a later compatibility decision changes it. A last-good result is eligible only
when its own published generation, coverage fence, and scope have not changed and its
own age is within that budget. A rolling-only refresh MUST NOT invalidate an otherwise
fresh all-time result, and a fresh global result MUST NOT make a stale account result
available.

An unknown account may return the existing exact zero response only when the account
catalog/coverage authority proves that the account has no historical projection data.
A known account without a complete snapshot, or any account with an unproven archive
intersection, is unavailable. Missing, changed, or identity-less archive manifests
are unavailable rather than empty.

### 6. Unavailable semantics

The following conditions map to the existing analytics unavailable boundary (including
the stable `analytics_backend_unavailable` code where the PG read boundary is used):

- replica not service eligible, baseline not ready, or projection state not ready;
- missing aggregate authority, missing exclusive cursor, or an unverified archive
  manifest/source generation;
- changed generation, coverage revision, source checkpoint, or cross-request cursor
  fence;
- invalid or incomplete fact classification, invalid numeric data, page/total budget
  overflow, or deadline exhaustion; and
- a global/account gap that can affect the requested selection.

There is no implicit SQLite fallback after PostgreSQL is explicitly selected. A
degraded data-quality marker is not a substitute for missing historical proof; the
response is unavailable until the fence is repaired.

## Deterministic parity fixture

The fixture uses only fields present in the current `analytics_facts` schema and the
operational `StatsTotals` contract. All rows are synthetic `codex_invocations` facts;
the first three intentionally share the same timestamp to exercise the tie-break key.
The `running` row is in the live tail, not an archived pending row.

| source row              | stats date   | occurred-at order | status      | success-like | terminal | failure class     | total tokens | cost |
| ----------------------- | ------------ | ----------------: | ----------- | -----------: | -------: | ----------------- | -----------: | ---: |
| `codex_invocations/101` | `2026-10-30` |    `00:00:00.000` | `completed` |         true |     true | `none`            |          100 | 1.25 |
| `codex_invocations/102` | `2026-10-30` |    `00:00:00.000` | `http_500`  |        false |     true | `service_failure` |           30 | 0.50 |
| `codex_invocations/103` | `2026-10-30` |    `00:00:00.000` | `http_401`  |        false |     true | `client_failure`  |           10 | 0.10 |
| `codex_invocations/104` | `2026-10-30` |    `00:00:01.000` | `running`   |        false |    false | `none`            |            5 | 0.05 |

For either one page or a continuation split after row 102, the exact historical-plus-
tail result is:

```text
totalCount     = 4
successCount   = 1
failureCount   = 2
totalTokens    = 145
totalCost      = 1.90
nonSuccessCost = 0.60
```

`usageBreakdown`, in-progress fields, maintenance, and `dataQuality` are not inferred
from this fixture; each needs its own authority. A page replay under the same fence
must produce the same result, while a changed fence must produce unavailable/restart-
required rather than a fifth or duplicated contribution.

## Minimal implementation plan

1. Add a pure Summary All aggregation/classification helper and deterministic unit
   tests for the fixture above, including same-timestamp keyset continuation,
   exactly-once merge, archived-pending classification, and invalid classification
   fail-closed behavior.
2. Define an internal `SummaryAllReadContext` and opaque cursor carrying the scope,
   aggregate generation, coverage fence, source checkpoint, high watermark, and last
   key. Validate page row/byte and total-tail budgets before materializing data.
3. Select or build a separately reviewed compact aggregate authority that contains the
   operational fields. Do not treat the current long-term daily/hourly rollups as
   sufficient, and do not add a schema or rollup in this design-only change.
4. Implement the one-transaction historical-prefix plus bounded-live-tail reader.
   Add deterministic tests for missing archive proof, global/account isolation, stale
   last-good snapshots, changed fences, oversized rows, and tail budget exhaustion.
5. Only after parity and fail-closed tests pass, decide in a separate compatibility
   change whether and how a selected PostgreSQL backend may serve the intended read
   surface. That change must preserve `StatsResponse`, avoid implicit fallback, and
   include its own schema/rollup review if a compact authority is required.

## Consequences

The contract makes “bounded” mean bounded work and bounded materialization, not merely
“the database returns one aggregate row”. Exactness is preserved by a verified
historical boundary, a stable high watermark, a deterministic keyset cursor, and one
snapshot fence. The trade-off is explicit: without a compact authority containing the
operational Summary fields, a PostgreSQL Summary All implementation cannot be admitted,
even though long-term daily rollups may still serve their separate overview/series
contract.

This ADR intentionally does not change the public response, PostgreSQL schema, rollup
columns, source fact derivation, or routing. It also does not establish pressure,
throughput, scale, production-data, or p95 evidence.

## Related specifications and decisions

- [`docs/specs/pg18-analytics/SPEC.md`](../specs/pg18-analytics/SPEC.md)
- [`docs/specs/pg18-analytics/IMPLEMENTATION.md`](../specs/pg18-analytics/IMPLEMENTATION.md)
- [`docs/specs/5k89c-long-term-usage-analytics/SPEC.md`](../specs/5k89c-long-term-usage-analytics/SPEC.md)
- [`ADR 0002: Exact summary projection contract`](0002-exact-summary-projection-contract.md)
- [`ADR 0004: Summary archive publication proof`](0004-summary-archive-publication-proof.md)
- [`ADR 0010: Summary coverage fence and snapshot authority`](0010-summary-coverage-fence-and-snapshot-authority.md)
- [`ADR 0014: Durable summary coverage recovery`](0014-durable-summary-coverage-recovery.md)
- [`ADR 0051: Optional PostgreSQL analytics offload`](0051-pg18-analytics-offload.md)
