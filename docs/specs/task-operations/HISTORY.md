# 任务运维运行观测与生效计划 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- The topic is active. It extends the existing maintenance task API and page while preserving legacy task fields and maintenance-history storage.
- Current runtime snapshots remain process-local. A process restart marks any execution without a confirmed terminal event unknown; persisted execution and deferral intervals remain historical records and do not resurrect a live worker.
- The additive runtime and catalog fields are compatible with consumers that ignore unknown JSON properties. PATCH clients using existing `enabled`-only requests retain their behavior.

## Replacements / Background

- ADR 0024 records the accepted decision to separate actual execution observation from durable history and to centralize effective schedule policy in a capability catalog.
- The diagnostic solution records the root cause: worker defaults were not present in the database projection, while the detail page rendered an empty override as a fixed interval.
- The task-overview requirements extend this boundary with separately labelled queued requests and admission-deferred tasks, stable identity colors, and a compact execution timeline whose lanes follow overlap rather than task identity. The deferral row concerns task admission rather than general runtime health; independent worker policies remain independent.
- ADR 0026 records the accepted persistent observation boundary: background collection continues without an open page, recent intervals survive service restart, and missing coverage remains explicit rather than reconstructed from request times or aggregate metrics.
- The task execution chart's visible rolling window is 12 hours. The timeline API continues to support windows up to 24 hours, and shortening the presentation window does not reduce persisted history retention.
- The performance repair also removes the persistent workload legend from catalog background rows. The P/D/C/status summary remains available in standalone charts, while background rows keep a compact accessible detail control so the full point record is still available on pointer or keyboard activation.
- The dense mixed-history follow-up replaces static/live pair enumeration with binary cross-index overlap counts and keeps live ordering linear per clock update alongside the strict endpoint semantics. Focused live detail text now survives clock ticks through the existing cache and only recomputes on revision changes. Background detail popovers render at the document layer, move keyboard focus into the detail and return it on Escape in both chart modes, and close when filtering hides their owning row; equal-revision workload events retain the current cached trend.
- Zero-length waits remain renderable but are excluded from overlap counts and expanded details. Dense demo cursors now carry the scene identity and reset cleanly when a caller changes scenes between page requests.
- Static history now receives a stable combined overlap-count map while the indexed live boundaries remain unchanged, avoiding per-second React work for unchanged historical SVG; focused execution and deferral details bypass the hover debounce when a revision arrives, and regenerated same-name demo scenes carry a snapshot token so old cursors reset after a scene round trip.
- Live wait templates now sort only on snapshot revisions; one-second clock projection preserves that order and builds the uniform live-end index without a second sort.
- The removed catalog legend's metric identity remains available in the on-demand detail dialog through colored P/D/C labels, so background rows stay uncluttered without making the three chart series ambiguous after interaction.
- The 13,120-interval regression evidence uses stable desktop-dark and mobile-light captures plus an expanded dense group with inspectable run identities. The captures wait for theme-color transitions to finish; the owner confirmed and approved the set on 2026-10-07.
- Timeline pages use a stable `(startedAt, segmentId)` keyset because revision changes can move earlier rows out of the fixed-watermark result set between requests; legacy offset cursors trigger a full resynchronization. The server selects a 12-hour window after reading the initial watermark, so browser clock skew cannot exclude intervals already covered by that watermark.
- Live runtime data uses dedicated SSE snapshots. The timeline descriptor without `schemaVersion` remains the compatible `/v1` interval snapshot/delta contract; `schemaVersion: "2"` selects marker-only `/v2` notifications carrying `watermark` and `observedAt`. The browser uses v2, starts fixed-watermark HTTP baseline paging immediately after subscribing, then loads `afterRevision` deltas over the committed bounds without fixed-period polling. Baseline pages stay staged until complete and revision notices are coalesced while paging. A committed HTTP baseline is known data even if the first marker is pending. The browser advances the visible clock between events and presents connecting, reconnecting, and disabled states; after the observation grace period it freezes open state and labels it unknown.
- The implemented task-detail chart region combines recent-100 run metrics and Retention's seven-day backlog with Tabs, rendered empty chart frames and persistent legends. Per-task capabilities separate complete pending population, discovered eligible candidates and committed work; absent measures remain absent.
- The two existing workload views use the shared segmented control and short labels “次数 / 时间”. The latter denotes Retention's seven-day backlog; this naming does not introduce another run-hour filtering mode. Mock runs conserve pending work across arrivals and committed processing, retain zero-commit and partial-commit failures, and do not infer processing from a skipped attempt.
- The owner requires the workload-chart header to place “运行趋势” at the left and the view Tabs at the right of the same row across desktop and mobile. Keeping only the Tab labels on one row does not satisfy this placement requirement; the heading and control must share the row to preserve chart space.
- The run metric view uses overlapping areas sharing a zero baseline. Where the same-unit candidate sets are nested, visible bands correspond to C, D−C and P−D while boundaries and Tooltip values remain C, D and P. Mixed units or unproven containment do not authorize difference bands or overall progress.
- Workload samples use the asynchronous maintenance recorder, remain independent of page lifetime, and protect each task's latest 100 attempts and unconfirmed running sample. Historical attempts retain their identity while unsupported metrics stay unknown. Detail SSE carries revisioned workload snapshots; stale sequences are ignored and recorder gaps are shown.
- The task timeline performance repair keeps the public transport and persistent contracts unchanged while moving formatting and completed-detail work behind stable caches, separating static history lanes from one-second projection, and replacing per-bar overlap scans with indexed queries and merged coverage subtraction. The dense demo scene and Storybook state provide fixed 13,120-interval evidence, including 6,000 waits, two reasons, open intervals, and keyboard/pointer detail activation.
- Root and child in-memory overlays retain active attempts and prune previous terminal samples when a new attempt begins. Coverage repair records each successfully committed bucket before continuing; later failure or cancellation preserves those confirmed counts in its final asynchronous snapshot.
- Processing speed uses at most 20 complete ended attempts and actual wall-clock span. Backlog estimates require at least five fresh same-range exact snapshots within 24 hours, a 60-second minimum span, positive net decline, enabled task state, and no coverage gap. Skips contribute zero only to the rate window and do not fabricate chart metrics or actual start times.
- The public workload response additions and maintenance-store workload table are forward-compatible. Timeline v1 consumers keep their interval-row contract; v2 is additive and supplies marker-only revision notifications for HTTP pagination. Existing request-time and duration meanings, task configuration, and stored colors remain unchanged.
- ADR 0030 supersedes ADR 0029's in-place v1 replacement choice and keeps ADR 0026's persistence and collection boundaries unchanged. Continuous background collection, maintenance-store ownership, restart behavior, retention, and explicit missing-coverage semantics remain unchanged.

## Current Delivery Facts

- The owner selected workload counts, rather than execution duration, for task-catalog row backgrounds and approved expanding reliable task-specific measurement coverage. The current candidate implements recent-run summaries, a rolling 24-hour window capped at 200 attempts per task, bounded visible-row loading, and the agreed collectors while preserving the detail contract's recent-100 run-order view. Already removed measurements remain historical unknowns; canonical visual assets remain owner-gated until the mock-only candidate screenshots are confirmed.

- The accepted implementation includes process-local runtime observation, separate live dispatcher and admission wait lists, stable persisted task colors, a restart-safe execution/deferral timeline, bounded revision-based reads, and durable task workload samples, alongside the 37-entry capability catalog, safe schedule editing, reset-to-default semantics, combined filters, and responsive detail views.
- The catalog computes default policy metadata without writing schedule overrides. Existing unsupported overrides remain readable and require an explicit reset; `enabled` is preserved when overrides are cleared.
- Legacy run-history request timestamps and durations retain their previous meanings; the implementation does not infer actual execution start times from them.
- The owner confirmed the mock-only desktop/mobile timeline and SSE connection-state evidence on 2026-10-02. Canonical assets are stored in `docs/specs/task-operations/assets/`; the mobile capture keeps the 12-hour chart compact without row labels.
- The owner confirmed all six workload-chart rectification screenshots on 2026-10-04. The accepted Storybook evidence replaces the earlier workload images and covers shared single-row Tabs, Retention's time view, mobile chart space, hidden-pending rescaling, and a running task without counters.
- The owner confirmed six header-alignment screenshots on 2026-10-05 and authorized Spec and PR reuse. They replace the prior canonical images and demonstrate the heading and view Tabs sharing one row with opposite-edge alignment on desktop and mobile.
- The owner confirmed the catalog background screenshots on 2026-10-06 and authorized visual-evidence submission. The final assets show the P/D/C background spanning each task row on desktop and mobile; a follow-up rendering correction closes filled areas at observed endpoints, removing the false diagonal edge caused by fixed chart-boundary closure.
- PR #1074's reconnect Storybook fixture allows 750 ms before the simulated disconnect while preserving its connection-state assertions and timer cleanup. The test-only correction passed current-head CI at `62354bb8`; it does not change production SSE behavior or the accepted workload-chart evidence.
- The final timeline repair candidate at `aeb63f51` keeps the on-demand P/D/C metric mapping inside the detail dialog after removing the persistent catalog legend, refreshes focused details immediately on revision, and retains the snapshot-bound dense demo cursor guards. It also keeps keyboard focus stable while a visible detail dialog repositions on resize or scroll. The workload background stays behind catalog text, uses its own observation time only for snapshots older than the rolling window, and otherwise keeps the real-time clock moving; the dense production demo visibly renders its P/D/C chart. The candidate was merged onto `origin/main` at `1f3881075b1fe749e964dda3ec66ab10b710c797`; current Web unit, Storybook focus, typecheck, lint, production build, demo build, and Chromium demo sampling are bound to the resulting head. The owner confirmed the fresh desktop and mobile catalog captures on 2026-10-09, and the canonical assets are stored under `docs/specs/task-operations/assets/`.

## Related Changes

- [PR #1074: durable workload trends and evidence-based estimates](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1074)

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/adr/0026-durable-task-execution-and-deferral-timelines.md`
- `docs/adr/0029-managed-task-timeline-http-pagination-and-sse-revision.md`
- `docs/adr/0030-versioned-managed-task-timeline-sse-compatibility.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`
- `docs/specs/task-operations/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
