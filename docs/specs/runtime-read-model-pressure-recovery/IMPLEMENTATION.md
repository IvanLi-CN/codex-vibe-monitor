# Runtime Read-Model Pressure Recovery - Implementation

> Canonical spec: `./SPEC.md`.

## Maintenance contention boundaries

REQ-RMPR-007 defines background invocation-range preparation and fixed recovery diagnostics without changing persistent allocation or the request wait. The shared producer interface is established in `observability::contention`. Startup reuses the bounded 48-hour activity-index read to select at most 128 existing conversation owners; the background loop prepares the current UTC hour and the next hour only within its final 60 seconds. Speculative entries yield to real allocation and retire before demand entries; a successful refill drives the remaining eligible batches without another request. Reservation transactions retain their native connection and admission through explicit settlement, including cancellation of their finish waiter. Stateful and real-file cancellation regressions are required in the integrated candidate's GitHub validation, alongside current review evidence. Production index definitions and fixed query plans remain separate read-only deployment evidence; missing evidence does not authorize index installation.

The live conversation-statistics query now removes the outer key column's affinity from its correlated comparison so the existing normalized-key expression index can serve `MAX(id)`. The production builder is reused by exact-reference and strict named-index EXPLAIN regressions, including correct fallback without the optional index and live-row changes across generations. Summary recovery adds bounded historical, live-tail and publication-conflict diagnostics plus a real-terminal interleave regression; it does not replace the publication algorithm or prove that a production `source_gap` has recovered. Current integrated acceptance remains separate from the child query-plan evidence.

Missing speculative owners are removed under the memory lock before notification, even when real demand promoted the entry during its read. Demand retries ordinary initialization within the existing request deadline instead of inheriting a stale preparation error. A gated regression deletes a selected owner, polls real demand through promotion and resumes the seed read; the previous failure assignment returned an error for this schedule. Explicit index dry-run inspection compiles the actual statistics builder and emits indexed, fallback or unavailable diagnostics; a fixture regression inspects before and after index preparation and verifies that repeated scraping adds no observations. The earlier unused diagnostic variants produced neither observed plan event.

REQ-RMPR-006 / VER-RMPR-006 bind the producer, bounded confirmation, optional-index CLI and exact-query interfaces in [maintenance contention boundaries](../../design/maintenance-contention-boundaries.md). The bounded producer coalesces task/control-generation obligations in memory; the independent confirmation owner applies short conditional writes and capped jittered backoff without resetting an older retry deadline. The terminal writer submits that memory notification and retains rejected obligations instead of awaiting maintenance-database confirmation. Integrated compatibility, cancellation and controlled CI evidence must bind the delivered candidate; implementation descriptions alone do not establish performance acceptance.

The parent implementation adds explicit ownership-aware `prepare-performance-indexes`, per-index result reporting, a shared total budget, and native interruption/rollback/close before admission release. It shares the normalized prompt-cache key expression with the live-row MAX query and adds reference/plan regressions without changing the returned fact. Legacy long-term interval migration advances an independent consumer cursor atomically with each canonical page using existing indexes, retaining invalid sources and revisiting valid late earlier IDs. These source changes and their resource-bucket regressions remain pending current-head CI and formal review. Summary retains its independent rolling and historical owners: a published Bootstrap fallback routes live-only changes through bounded tail reconciliation and reopens cold hydration only for changed historical coverage. No request-side SQL fallback is added.

The fallback regressions commit their new live record through the actual terminal batch writer, including the atomic source descriptor, then exercise both retained descriptors and a durably compacted tail. Both reconstruction queries append predicates to the common invocation query's existing WHERE clause. A direct SQL insert without that descriptor is an unproven source change and cannot serve as an exact terminal fixture. The regressions retain independent assertions for exact recent counts, an unavailable historical selection, zero repeated cold hydration for live changes, and one bounded retry after historical coverage changes. Their source remains in the lightweight resource bucket and retains exclusive timing slots.

Durable descriptor reconciliation compares the captured immutable base against the current published revision and carries a separate durable target into publication. A later source commit prevents checkpoint/fence advancement; a subsequent bounded pass restores the complete tail. A reconstructed overlay awaiting fenced publication cannot take the ordinary freshness-only shortcut, which would skip the remaining source tail. The publication transaction acquires its short write reservation before the final fence read, avoiding a read-to-write upgrade race. A deterministic interleave regression inserts a second real terminal after reconstruction and requires the first checkpoint to remain pending before both rows become exact, without historical rehydration.

The prepared live-tail publication also takes a non-blocking derived-write admission after acquiring its pool connection. A waiting terminal leaves its reconciliation checkpoint pending for a later turn. The native cleanup owner retains the derived-write and background permits across cancellation of an active SQLite statement or its finish waiter. A deterministic P1-admission regression and file-backed native-worker cancellation cases are added to the matching resource buckets; their current integrated GitHub execution and review are pending.

Backfill integration adds two file-backed regressions: terminal facts remain durable while an independent maintenance writer is locked, and a real sixty-second lock window receives repeated memory notifications while allowing at most sixteen actual confirmation attempts. The latter records its notification and attempt counts before unlocking and verifies that one durable confirmation covers all accepted notifications. Scheduler/control-generation regressions remain separate from this real-lock case. These prepared tests run only in the GitHub archive-file-io profile and do not establish acceptance until current-candidate CI passes.

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
- Dashboard compacted-source recovery no longer treats the journal retention
  bound as a full-table replay limit. When a compaction proof intersects the
  Dashboard day windows, the `SummaryCoverageRecoverySupervisor` creates an
  additive checkpoint for fixed Shanghai `today`, `yesterday`, and
  `previous7d` ranges. It pages canonical terminal rows by `(occurred_at, id)`
  with a maximum of 512 rows and records aggregate, usage, bounded record
  identity, cursor, fence, and state in SQLite without copying invocation
  payloads.
- Each Dashboard recovery page is committed atomically with its tuple cursor;
  the candidate query measures text byte lengths before loading source fields,
  then materializes only the selected rows. The page byte budget is 8 MiB, the
  write-transaction budget is 250 ms, and a worker turn is capped at 2 s. An
  oversized error message is represented by a stable classification sentinel
  after canonical SQL has resolved its failure class, so one pathological row
  advances as a singleton instead of permanently failing recovery. Pressure or
  contention records `deferred`, a restart resumes the last committed page, and
  a changed generation fence clears unpublishable staging before rebuilding.
  Transaction timeout cancellation awaits rollback before releasing the
  background admission. Only completed staging that still matches the fence is
  published into the in-memory Summary projection.
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
- V2 page progress is separate from coverage authority. Every completed
  manifest has a durable `SummaryCoverageObligation`; a backfill outcome,
  including `Complete`, cannot hide that obligation. Only the identity-bound
  `SummaryArchiveSnapshotV2Proof` marker, committed after complete semantic
  validation, resolves the obligation and advances the affected coverage fence.
  Page or manifest mutation revokes the marker, while legacy terminal outcomes
  become explicit range-local gaps.
- Snapshot V2 backfill pages use the proof order `(UTC occurred_at, id)` rather
  than the generic historical-rollup ID pager. The durable outcome stores the
  cursor version, next timestamp, row ID and retry attempt. Legacy ID-only
  progress is reset when no complete V2 proof exists; transient source/SQLite
  failures retain the last verified cursor, while semantic or manifest identity
  failures are quarantined until the manifest SHA changes. Its stored hash state
  resumes standard SHA-256 at the committed byte offset and rejects a changed
  archive fingerprint before it can adopt unverified pages.
- `SummaryCoverageRecoverySupervisor` is now the sole runtime owner for
  unfinished AllTime checkpoint pages and Legacy Snapshot V2 backfill. It
  advances one generation-fenced proof page, gives 30-day-intersecting Snapshot
  candidates priority through a due queue before the archive-ID sweep, and
  finalizes only after exact proof. It receives a bounded cadence turn without
  Summary HTTP/SSE ownership and takes a database-pressure permit before any
  recovery I/O. The regular Summary
  refresh path no longer invokes the generic AllTime builder or paged raw
  hydration; pressure, restart, and SQLite lock defer preserve the last
  committed cursor while recent Projection selections stay available.
- HTTP and Summary SSE compose the immutable base and journal from hub memory.
  A current selection that would require rank replacement, or a range intersecting
  an independent archive/source coverage gap, is unavailable rather than
  approximate. A selection intersecting only a terminal `DeltaGapProof` or an
  eligible broad Source Journal compaction proof serves the published last-good
  base plus acknowledged overlay with explicit `dataQuality` degraded status;
  scoped source identity gaps and missing overlays remain unavailable, and
  `/api/stats` uses the same all-time memory path.
- Summary archive replay-marker repair is ordered by dedicated partial SQLite
  indexes, and replay admission requires a nonblank marker matching the current
  archive SHA; NULL or blank markers fail closed. Additive repair reads at most 128
  pending candidates per background turn. Stale-marker or incomplete-materialized recovery uses the established 4,096 completed
  authoritative-batch ceiling per bounded replacement transaction and persists an
  archive ID cursor between pages. Force repair persists source invocation IDs
  across pages, clears safe rollup buckets before rebuilding when a materialized
  authority bucket is missing, and restores live rows without duplicate totals.
  Incomplete recovery persists its live cursor without publishing completion. The
  backfill entrypoint resumes force pages whenever the durable archive cursor or
  incomplete marker exists, even if the current page repaired the marker predicate;
  it returns before additive replay until the full repair publishes completion.
  The request path never runs this proof or a broad historical scan.
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
- Summary Startup Recovery Gate keeps raw legacy-mirror identity work outside the first Summary build. Bootstrap retains unavailable or mismatched sources as `unknown` selection-local fail-closed coverage, without opening, inflating, or hashing legacy archive files. The generic legacy-mirror backfill defers while no Projection exists, then resumes its durable cursor after cold hydration publishes. Health readiness and HTTP/SSE remain independent of this background I/O and retain the existing unavailable contract until a Projection is published.
- Summary startup publishes a Bootstrap Projection after bounded current and rolling/calendar hydration. Paged archive manifest metadata can establish compact proof or a range-local gap during Bootstrap, but paged raw archive SHA/file/record hydration is exclusive to all-time reconciliation. The durable global/account all-time checkpoint records the complete generation fence, independent manifest-proof and rollup seek cursors, plus committed global/account aggregate accumulators. Each page commits its result with its cursor, so restart resumes the matching generation rather than rebuilding history from zero; only a completed scope atomically publishes its exact all-time response.
- Historical live coverage follows the same staged availability boundary. Bootstrap and ordinary refreshes never enter the historical live group-by; the dedicated coverage Supervisor owns any bounded proof recovery and publishes it through the generation-fenced checkpoint/overlay path. Rolling reads only the ID delta inside an unchanged proof overlap and verifies the small moving tail; it neither repeats the full group-by nor opens a request-time fallback. A late historical ID becomes an hour-local unavailable proof until the Supervisor replaces it with complete coverage, while the fresh current/recent Projection remains published.
- Bootstrap does not publish speculative zero-valued all-time account entries, and a bounded current-rank cutoff does not invalidate separately exact rolling ranges. When a refresh observes a finite archive gap, it publishes the independently exact selections with that gap marked unavailable while retaining an exact all-time last-good response where available. Account-manifest or account-replay gaps remain account-scoped so a globally proven compact rollup stays available; a gap without finite coverage proof remains broadly unavailable. The final all-time assembly has its own bounded background deadline so materialized current boundaries can converge without extending the startup gate.
- Projection freshness renewal compares the complete durable generation fence before it extends a published rolling response. A matching fence renews only the in-memory freshness lease; a changed live watermark, global/account rollup cursor, archive manifest high-watermark, or settled terminal sequence takes the normal bounded recovery path. An all-time task that observes a changed fence cancels before publication, releases its refresh lock, and hands off to the live-tail reconciler or Historical Coverage Supervisor; staged finalization checks the fence before and after its atomic swap.
- Live-tail reconciliation now uses a durable `(row_id, invoke_id, occurred_at)` identity. ACKs and restart replays already represented by the immutable Projection are absorbed idempotently; descriptors without a durable row identity retain a broad fail-closed proof. A versioned checkpoint fixes the base revision and target watermark, performs bounded descriptor reconstruction under the pressure permit, and records no source payload. Its readiness state is exposed only through no-payload health fields and structured stage telemetry.
- Summary Delta Journal retains a broad fail-closed proof instead of evicting an older scoped proof when its proof budget saturates, while preserving whether the broad proof came from source loss or terminal-only overflow. Terminal-journal and shutdown replay records become exact committed rolling overlays with their own bounded residency, because their prior process-local dashboard sequence is not valid after restart; replay does not trigger full live admission.
- The Dashboard recovery API keeps HTTP 503 for `today`, `yesterday`, and
  `previous7d` until the compacted snapshot is ready, and adds stable
  `summary_recovery_pending` or `summary_recovery_failed` JSON codes while
  retaining the human-readable message. System status exposes only state,
  processed-row count, and an optional error code; the web client renders the
  coded pending/failed state rather than indefinite loading or zero-valued
  success. A pending coded response schedules bounded 1s/2s/5s/10s/30s HTTP
  retries, so completion is observed without waiting for remount or day
  rollover.
- Overflowed boundary manifests localize legacy missing coverage with the immutable Shanghai `month_key` partition. An unknown partition that overlaps the supported horizon remains range-local unavailable; an old disjoint partition does not poison current or rolling availability. Historical persisted-live coverage is grouped by source hour and account, retaining bounded terminal identity proof only where an SSE overlay needs it, so aggregate historical cardinality cannot abort Bootstrap.
- Promotion policy: checkpointed; every included Ticket requires observed evidence after owner-confirmed manual deployment.

## Compatibility and Migration Records

- The invocation-range recovery component has separate [API/version impact](assets/invocation-range-version-impact.json) and [durable reservation/re-entry](assets/invocation-range-persistent-state.json) records. They preserve the deployed v4.8 reservation representation and explicitly leave final integrated verification pending.
- Public API and persistent-state impacts are recorded separately in `assets/version-impact-record.json`.
- SQLite index installation, bounded replay writes, idempotent re-entry, recovery, and excluded deployment/allocator scope are recorded in `assets/persistent-state-migration-record.json`.
- The current implementation candidate is `7ea3b6415c4d744623005a12ff434d1157c067a0` on `th/summary-source-recovery-batch`, based directly on the locked development baseline `207e6d6341240dd722d15a9e853485bed5e441a9`. Current required-lane evidence and the local validation boundary are tracked in `assets/current-candidate-validation.json`; CI and Tier 4 review must bind to the pushed branch head.
- Summary replay-marker repair treats NULL or blank marker SHA values as legacy-readable proof, reopens nonblank stale SHA values, and forces a full existing-rollup rebuild when a materialized archive lacks either Summary marker; incomplete unmaterialized archives remain additive and bounded.

## Approved Recovery Boundary

- A Summary-affecting terminal transaction appends one compact descriptor in
  the existing transaction. Normal terminal traffic keeps the current
  in-memory delta fast path; restart recovery reconstructs only the
  descriptor's bounded keys and does not perform complete live admission.
- The global durable cursor checkpoint is monotonic and is written only after a
  bounded tail has been reconstructed. `SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES`
  remains a `10,000 entries / 64 MiB` journal-retention bound; it is not a
  Dashboard recovery admission threshold. Compaction preserves a
  range/account/current-rank proof, and descriptors/proofs contain no raw text
  or duplicate Summary rows.
- Dashboard recovery uses a separate additive checkpoint and staging set. It
  retains only normalized aggregates, `(occurred_at, id)` progress, generation
  fence values, and `list_limit_max`-bounded record identities, so a 10,001-row
  source range remains recoverable without a production database copy.
- Archive maintenance exposes the normalized Snapshot page writer and cleanup
  proof gate in main SQLite. Snapshot proof includes manifest identity,
  coverage and payload SHA; a seek-paged legacy backfill can reuse the writer
  for readable authority, while source loss remains a finite unavailable range.
- The all-time coverage fence carries independent global and account revisions
  across manifest, archive replay and Snapshot V2 proof versions. Archive
  rollup materialization advances the fence through its replay marker; hot/live
  rollup writes change only the live-tail cursor. A terminal tail changes only the live-tail cursor
  and bounded overlay; it never cancels an in-flight historical page. The
  backfill outcome index stores only disposition, failure class, next probe and
  the next page/row seek key, never archive payload or source text. Archive
  hashing and page iteration check the bounded maintenance budget; a deferred
  archive resumes after its last verified page without monopolizing the worker.
- Snapshot V2 completion compares the durable coverage fence with the published
  Projection and publishes a generation-fenced immutable Coverage Publication
  Overlay when verified contributions differ. The overlay reduces only compact
  V2 totals and availability state; it never starts generic RollingDelta, live
  admission, or paged raw hydration. Ready checkpoints skip repeated
  finalization, completed usage proof skips its full rollup scan, recent retry
  outcomes obey `next_probe_at`, and an unchanged completed backfill checkpoint
  performs no progress write.
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
