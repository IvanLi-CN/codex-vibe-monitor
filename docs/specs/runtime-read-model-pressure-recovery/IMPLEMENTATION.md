# Runtime Read-Model Pressure Recovery - Implementation

> Canonical spec: `./SPEC.md`.

## Current Status

- Lifecycle: active canonical-classification recovery initiative.
- Implementation: existing `failure_class` compatibility writers and backfill are not yet sufficient to make every read model consume one revisioned durable classification fact. Canonical terminal materialization, immutable-archive overlay coverage and shared Summary/rollup consumer migration are the first delivery boundary.
- Summary admission foundation keeps the bounded newest-N `current` view separate from rolling exact-boundary records, while enforcing one resident preview-byte budget and a monotonic omission boundary across runtime overlays. Archive coverage-end proof now rejects only a global newest-N that an unadmitted archive can affect; account newest-N remains unavailable until account-specific archive proof exists. Historical persisted terminals also retain independent global/account rollup proof before Summary or SSE removes their overlay. This restores honest high-cardinality admission without claiming canonical classification materialization is complete.
- Promotion policy: checkpointed; every included Ticket requires observed evidence after owner-confirmed manual deployment.

## Delivery Boundaries

| Delivery slice                                 | Purpose                                                                                                                                                                        | Integration order                                           | Completion evidence                                                                                    |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Canonical classification and Summary admission | Materialize one revisioned terminal outcome across live/archive records, then build compact exact Summary snapshots from it without raw-payload retention or request-time I/O. | Wave 1                                                      | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |
| Pressure defer and startup backfill            | Keep gate defers in an in-memory scheduler deadline/event path and distinguish them from real lock failure.                                                                    | Wave 2 after the Summary baseline is restored               | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |
| P2 writer admission                            | Yield hourly rollup, projection flush and task finalization before P1 terminal durability under pressure.                                                                      | Wave 3, after pressure is observed                          | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |
| Long-term legacy migration                     | Replace legacy full-window migration scans with cursor/seek microtransactions.                                                                                                 | Wave 4, after pressure and P2 writer admission are observed | child/integration CI, checkpoint release, owner-confirmed deployment, 900-second read-only observation |

## Integration Rules

- The current Initiative binds its own canonical-classification integration branch; child PRs target it and do not release directly.
- Canonical classification is the green baseline before pressure work, because all child CI profiles must exercise the same exact Summary behavior.
- P2 writer admission cannot start until pressure reaches its `observed` completion gate; long-term migration cannot start until both reach that gate.
- Existing timeseries writer work remains outside this Initiative. Draft #714 stays out of the integration branch and is only marked superseded after the successor long-term slice has passed its technical acceptance.
- Checkpoints publish GitHub artifacts only. Dockrev and srv-101 deployment, restart, rollback and write operations remain owner-only.

## Verification Ownership

- Canonical classification and Summary: transaction-atomic terminal materialization, bounded legacy live cursor, immutable archive overlay coverage, rollup recomputation, cross-reader exact-response comparison, >legacy-cardinality admission, bounded recent-index overflow boundaries for rolling/account reads, HTTP SQL/file counters, freshness and last-good behavior.
- Pressure: one in-memory scheduler defer deadline per cooldown, no SQLite pre-read or no-op task-run write; Account Activity V2 coverage repair owns one admitted permit across its durable due check, underlying repair, and every post-repair progress operation; repair and retry-progress `BUSY`/`LOCKED` regressions assert one pressure event, no outer task-run audit or generic retry, and no early next-task SQLite access, while a non-lock coverage error remains audited and retried; eligibility wakes recheck durable due state; and real-lock cooldown/backoff separation happens before permit release.
- Long-term: query-plan assertion, cursor persistence, 512-row transaction cap, pressure/cancel recovery and P1 priority.
- Observation: `$srv-101-ops` is read-only and only starts after the owner confirms the exact released version is deployed.
