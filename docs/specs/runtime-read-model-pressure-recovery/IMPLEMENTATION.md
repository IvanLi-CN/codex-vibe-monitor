# Runtime Read-Model Pressure Recovery - Implementation

> Canonical spec: `./SPEC.md`.

## Current Status

- `SummaryDeltaJournal` retains compact terminal deltas only after the SQLite
  batch writer has acknowledged the source record. Its 10,000-entry / 64 MiB
  cap and monotonic cursor make the rolling tail bounded.
- `RollingDelta` renews a published rolling Projection from that continuous
  journal instead of entering complete live admission. Its durable tail is
  reconstructed from compact identity descriptors after restart; source changes
  without descriptor evidence use a bounded legacy tail read plus a scoped proof,
  never a complete live-admission fallback.
- ADR 0009's durable Source Change Journal and SQLite-backed Summary Archive
  Snapshot are implemented as the recovery boundary. The journal appends one
  identity-only descriptor inside each terminal source transaction, compacts
  under the 10,000-entry / 64 MiB bound with a durable proof, and the Snapshot
  API verifies manifest identity, coverage and payload SHA before cleanup can
  retire an authoritative archive.
- ADR 0010 separates the immutable Summary Coverage Fence from the bounded live
  tail cursor. Bootstrap and rolling refreshes defer raw archive boundaries to
  AllTime recovery, and only semantically verified Snapshot V2 pages can satisfy
  archive cleanup proof; V1 pages remain backfill input only.
- Legacy Summary Coverage Recovery now runs as a seek-paged maintenance window
  with its own durable cursor and per-manifest outcome. It first reuses an
  already verified V2 page without opening raw source, otherwise validates the
  archive SHA and materializes bounded V2 pages. Missing, unreadable or
  mismatched authority is recorded as a finite unavailable outcome and remains
  retryable; a budget boundary leaves the cursor at the last committed page.
- Snapshot V2 backfill pages use the proof order `(UTC occurred_at, id)` rather
  than the generic historical-rollup ID pager. The durable outcome stores the
  cursor version, next timestamp, row ID and retry attempt. Legacy ID-only
  progress is reset when no complete V2 proof exists; transient source/SQLite
  failures retain the last verified cursor, while semantic or manifest identity
  failures are quarantined until the manifest SHA changes.
- `SummaryCoverageRecoverySupervisor` is now the sole runtime owner for
  unfinished AllTime checkpoint pages and Legacy Snapshot V2 backfill. It
  advances one generation-fenced proof page, gives 30-day-intersecting Snapshot
  candidates priority, and finalizes only after exact proof. The regular Summary
  refresh path no longer invokes the generic AllTime builder or paged raw
  hydration; pressure, restart, and SQLite lock defer preserve the last
  committed cursor while recent Projection selections stay available.
- HTTP and Summary SSE compose the immutable base and journal from hub memory.
  A current selection that would require rank replacement, or a time/account
  range intersecting a `DeltaGapProof`, is unavailable rather than approximate.
- Lifecycle: active canonical-classification recovery initiative.
- Projection readiness is now exercised as two independently timed facts: current and rolling/calendar selections must be exact-ready within 30 seconds, while all-time exactness may converge through the generation-fenced checkpoint within 1800 seconds.
- Cold maintenance retries now retain the 30-second Bootstrap mode until the hub has atomically published its first immutable Projection; only published Projections use the four-second Rolling refresh path. Bootstrap telemetry records the current archive admission, runtime overlay, Projection materialization and generation-fence snapshot stages without source content.
- The representative-scale fixture uses a fixed seed, at least 214 MiB of raw source text, and an independent normalized JSON oracle. It is a CI-contained PR/Main gate; Release consumes the successful CI Main result without production-copy validation or an external receipt.
- Implementation: existing `failure_class` compatibility writers remain a
  separate migration boundary; Summary source recovery now has transaction
  descriptors, bounded restart reconstruction and an archive Snapshot gate.
  Rollup/archive writers that lack descriptor hooks continue through their
  scoped legacy proof and background reconciliation path.
- Summary admission foundation keeps the bounded newest-N `current` view separate from rolling exact-boundary records, while enforcing one resident preview-byte budget and a monotonic omission boundary across runtime overlays. Archive coverage-end proof now rejects only a global newest-N that an unadmitted archive can affect; account newest-N remains unavailable until account-specific archive proof exists. Historical persisted terminals also retain independent global/account rollup proof before Summary or SSE removes their overlay. This restores honest high-cardinality admission without claiming canonical classification materialization is complete.
- Summary admission now evaluates a global archive against the requested newest-N cutoff instead of the configured maximum list size. A historical terminal is removed from a rolling or SSE overlay only when the matching scope has both totals and usage coverage; current-only persisted terminals that can affect exact repair are promoted into the rolling view. The rolling and current resident views preflight against one combined byte budget, so an overflow makes only the affected view unavailable.
- The required Summary admission delivery boundary separates canonical source-record admission from the shared resident preview-byte budget. Cumulative raw payload text must be processed through finite background source pages without consuming resident-preview capacity; when a source page cannot prove exact coverage, the Projection must publish each independent exact selection and mark only the intersecting rolling/calendar boundary or potentially affected `current` prefix unavailable.
- `current` admission is rank-complete: if a requested global rank is not resident and an unrepresented archive could supply it, the response is unavailable rather than shortened. Account `current` remains unavailable whenever any completed archive lacks account-specific representation, including a quiet account whose newest record is older than the global tail horizon. Manifest coverage endpoints are treated as inclusive source timestamps before archive rows are merged.
- Archive coverage keeps an exact manifest-end proof for newest-N admission while retaining bucket-expanded gaps only for rolling/calendar aggregation. Materialized archive boundary replay gaps therefore do not invalidate complete rollup interiors for yesterday/all-time, and replay coverage rejects stale manifest SHA identities while retaining the defined materialized legacy compatibility marker.
- Bounded archive manifests normalize their inclusive final timestamp into one exclusive range for raw replay and all-time compact-rollup proof, including one-row manifests. A raw current-candidate record-budget overflow is localized to current/range admission, allowing an otherwise complete rollup-backed snapshot to publish.
- Summary SSE cold start now follows the same memory-only availability boundary as HTTP: without a published Projection it returns `unavailable` instead of constructing a legacy SQLite baseline. Current-index pruning also keeps a compact-rollup-complete archive candidate out of the rolling omission boundary, so a bounded newest-N prefix cannot reject an independently exact rolling/calendar response.
- All-time raw fallback now treats archive replay as SHA-identified global/account coverage, so a stale marker for a replaced path cannot hide needed archive data. SummaryCurrent captures its Projection and terminal overlay in one hub-state snapshot, preserving an accepted terminal across a concurrent refresh swap; current admission accepts the selected cutoff immediately after an unrepresented archive's inclusive final-row endpoint.
- All-time account admission now requires the completed account-manifest refresh marker in both normal and paged paths; a partial observed account list cannot publish a fresh zero-valued archive account. The canonical replay writer stores the current completed manifest SHA with its marker, so an already replayed usage breakdown remains a valid read-side proof while a missing or replaced manifest fails closed.
- The approved archive recovery boundary treats `completed` invocation manifests as Summary-eligible only after database-enforced Archive Publication Proof. The implementation writes that proof atomically in retention and has no operator CLI or manual SQLite repair path.
- Legacy detail mirror reconciliation is a separate bounded startup cursor. It uses current-SHA, budgeted inflation, and page-by-page archive/live `(id, invoke_id)` proof before assigning `live_mirror`; the historical-rollup transaction never performs that file proof. Sparse historical segment ranges therefore converge without replaying duplicate canonical records, while any mismatch remains on the existing authoritative proof-recovery path.
- Summary Startup Recovery Gate captures the initial unknown-manifest ID high-watermark and performs bounded concurrent identity windows before the first Summary build. Each window commits only proven mirror roles, retains unavailable or mismatched sources as `unknown`, and advances independently of those conservative gaps. The generic legacy-mirror backfill defers while no Projection exists, and periodic Summary maintenance starts only after cold hydration publishes an exact Projection. Health readiness and HTTP/SSE remain independent of this background I/O and keep the existing unavailable contract until publication.
- Summary startup publishes a Bootstrap Projection after bounded current and rolling/calendar hydration. Paged archive manifest metadata can establish compact proof or a range-local gap during Bootstrap, but paged raw archive SHA/file/record hydration is exclusive to all-time reconciliation. The durable global/account all-time checkpoint records the complete generation fence, independent manifest-proof and rollup seek cursors, plus committed global/account aggregate accumulators. Each page commits its result with its cursor, so restart resumes the matching generation rather than rebuilding history from zero; only a completed scope atomically publishes its exact all-time response.
- Historical live coverage follows the same staged availability boundary. Bootstrap and the dedicated background recovery mode group the bounded historical interval by source hour and account, then publish the resulting generation-fenced proof with the Projection. Rolling reads only the ID delta inside an unchanged proof overlap and verifies the small moving tail; it neither repeats the full group-by nor opens a request-time fallback. A late historical ID becomes an hour-local unavailable proof until the background pass replaces it with complete coverage, while the fresh current/recent Projection remains published.
- Bootstrap does not publish speculative zero-valued all-time account entries, and a bounded current-rank cutoff does not invalidate separately exact rolling ranges. When a refresh observes a finite archive gap, it publishes the independently exact selections with that gap marked unavailable while retaining an exact all-time last-good response where available. Account-manifest or account-replay gaps remain account-scoped so a globally proven compact rollup stays available; a gap without finite coverage proof remains broadly unavailable. The final all-time assembly has its own bounded background deadline so materialized current boundaries can converge without extending the startup gate.
- Projection freshness renewal compares the complete durable generation fence before it extends a published rolling response. A matching fence renews only the in-memory freshness lease; a changed live watermark, global/account rollup cursor, archive manifest high-watermark, or settled terminal sequence takes the normal bounded rebuild path. An all-time task that observes a changed fence cancels before publication, releases its refresh lock and immediately hands off to that rolling rebuild; staged finalization checks the fence before and after its atomic swap.
- Summary Delta Journal retains a broad fail-closed proof instead of evicting an older scoped proof when its proof budget saturates. Terminal-journal and shutdown replay records become exact committed rolling overlays with their own bounded residency, because their prior process-local dashboard sequence is not valid after restart; replay does not trigger full live admission.
- Overflowed boundary manifests localize legacy missing coverage with the immutable Shanghai `month_key` partition. An unknown partition that overlaps the supported horizon remains range-local unavailable; an old disjoint partition does not poison current or rolling availability. Historical persisted-live coverage is grouped by source hour and account, retaining bounded terminal identity proof only where an SSE overlay needs it, so aggregate historical cardinality cannot abort Bootstrap.
- Promotion policy: checkpointed; every included Ticket requires observed evidence after owner-confirmed manual deployment.

## Approved Recovery Boundary

- A Summary-affecting terminal transaction appends one compact descriptor in
  the existing transaction. Normal terminal traffic keeps the current
  in-memory delta fast path; restart recovery reconstructs only the
  descriptor's bounded keys and does not perform complete live admission.
- The global durable cursor checkpoint is monotonic and is written only after a
  bounded tail has been reconstructed. The active tail remains bounded to
  `10,000 entries / 64 MiB`; compaction preserves a range/account/current-rank
  proof, and descriptors/proofs contain no raw text or duplicate Summary rows.
- Archive maintenance exposes the normalized Snapshot page writer and cleanup
  proof gate in main SQLite. Snapshot proof includes manifest identity,
  coverage and payload SHA; a seek-paged legacy backfill can reuse the writer
  for readable authority, while source loss remains a finite unavailable range.
- The all-time coverage fence is scope-local across manifest, rollup, replay and
  Snapshot V2 proof versions. A terminal tail changes only the live-tail cursor
  and bounded overlay; it never cancels an in-flight historical page. The
  backfill outcome index stores only disposition, failure class, next probe and
  the next page/row seek key, never archive payload or source text. Archive
  hashing and page iteration check the bounded maintenance budget; a deferred
  archive resumes after its last verified page without monopolizing the worker.
- Journal insertion adds no transaction or connection. Descriptor insertion,
  compaction and Snapshot writes emit only stage, count and byte telemetry;
  bounded reconstruction duration remains measured by the projection worker.

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

- Canonical classification and Summary: transaction-atomic terminal materialization, durable source-descriptor/proof atomicity, restart descriptor reconstruction, bounded legacy live cursor, immutable archive Snapshot coverage and cleanup gate, legacy Snapshot resume, rollup recomputation, cross-reader exact-response comparison, >legacy-cardinality admission, bounded recent-index overflow boundaries for rolling/account reads, HTTP SQL/file counters, freshness and last-good behavior.
- Pressure: one in-memory scheduler defer deadline per cooldown, no SQLite pre-read or no-op task-run write; Account Activity V2 coverage repair owns one admitted permit across its durable due check, underlying repair, and every post-repair progress operation; repair and retry-progress `BUSY`/`LOCKED` regressions assert one pressure event, no outer task-run audit or generic retry, and no early next-task SQLite access, while a non-lock coverage error remains audited and retried; eligibility wakes recheck durable due state; and real-lock cooldown/backoff separation happens before permit release.
- Long-term: query-plan assertion, cursor persistence, 512-row transaction cap, pressure/cancel recovery and P1 priority.
- Observation: `$srv-101-ops` is read-only and only starts after the owner confirms the exact released version is deployed.
