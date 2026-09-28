# Dashboard 对外调用时间线 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: aligned with the replacement contract; the target count path mounts only the invocation timeline.
- Lifecycle: active
- Catalog note: dashboard / invocation timeline / TTFT

## Implementation Coverage

- `REQ-DIT-001`, `REQ-DIT-004`: `src/api/slices/invocations_and_summary.rs`, `src/maintenance/hourly_rollups.rs`.
- `REQ-DIT-002`, `REQ-DIT-003`, `REQ-DIT-005`, `REQ-DIT-006`: `web/src/hooks/useInvocationTimeline.ts`, `web/src/features/dashboard/DashboardInvocationTimeline.tsx`, `web/src/features/dashboard/DashboardActivityOverview.tsx`.
- API normalization: `web/src/lib/api/core-foundation.ts`, `web/src/lib/api/feature-clients.ts`, `web/src/lib/api/types.ts`.
- Verification: Rust overlap, release, capacity, and background-cleanup tests; web timeline pagination, refresh scheduling/recovery, snapshot/window-state and API tests; lane assignment, layout, tooltip, scroll, and visible-row tests; Storybook interaction coverage; typecheck, lint, production build, Rust check, and responsive visual evidence.

## Coverage / rollout summary

- The endpoint is additive and read-only. It captures invocation and attempt row watermarks, freezes the folded records and runtime overlay in one read transaction, then publishes the bounded result into short-lived SQLite snapshot rows with a short `BEGIN IMMEDIATE` transaction using bounded multi-row batches. Subsequent requests read only those rows through the opaque cursor, so mutable source rows cannot drift between pages without holding the writer lock during the full scan. Snapshot entries expire after 30 minutes; a bounded cache rejects new first-page anchors when full instead of evicting a cursor that may still be in use.
- The timeline request carries the selected natural-day UTC bounds separately from the viewport. The server rejects scopes and viewports wider than 24 hours, clamps persisted and live boundary overlap to the preceding 24 hours, and keeps the existing 30-day duration safety check separate from query range selection. This prevents a short viewport from scanning a 30-day history while preserving long terminal durations for calls that begin inside the selected day.
- Each materialized snapshot has explicit row and serialized-payload budgets, and the active cache has aggregate row and byte budgets. Exceeding any budget returns the timeline's unavailable state and rolls back the partial snapshot; it never truncates the result, samples rows, or mounts the legacy aggregate chart.
- Once the client has traversed and merged every page for an `asOf`, it sends an idempotent `DELETE /api/stats/invocation-timeline/{asOf}`. The server immediately removes that token's in-process row/byte reservation and queues its durable rows for the existing 60-second P2-pressure-gated cleanup. The queue is bounded; overflow leaves rows for the normal bounded cleanup scan, and the 30-minute TTL remains the fallback for interrupted traversals or process loss. Release errors are diagnostic only and do not prevent the completed snapshot from being committed by the client.
- Materialization failures and canceled first-page requests use the same reservation-release and cleanup queue path. Their request guards do not perform synchronous or detached SQLite deletes; a canceled, incomplete traversal remains pageable only until its token is released or its TTL expires.
- Snapshot capacity rejections are classified by active-token, per-snapshot row/byte, and aggregate row/byte limits. Background cleanup logs active snapshot count/row/byte usage, queued release depth and overflow count, and each rejection counter so capacity exhaustion can be separated from query or database failures.
- The target `totalCount` path mounts `DashboardInvocationTimeline` without a legacy chart prop. Loading, unavailable, stale, disconnected, and pagination errors remain inside the timeline surface; non-target metrics and ranges keep the aggregate chart.
- The live hook traverses every page with one `asOf`, merges records by `(invokeId, occurredAt)`, releases the completed server reservation, and coalesces revisions while a request is in flight. SSE revisions and the 15-second poll share one scheduler with at most one traversal in flight and one automatic traversal per rolling 15-second interval; traversal spacing and retry deadlines use the same browser clock, with a small timer guard against sub-millisecond scheduling variance.
- Failed traversals retain and freeze the last good snapshot, then retry after 15, 30, and 60 seconds with ±20% jitter, capped at 60 seconds. A successful traversal resets retry state. A retry can continue during SSE disconnection; ordinary revision-driven updates wait for the live connection to recover. User viewport and scope changes still load immediately.
- The live hook keeps the requested viewport separate from one atomically committed `{data, window}` snapshot. Automatic live-window advances continue rendering that committed pair until the replacement succeeds; a failed refresh leaves it visible and stale. User zoom/pan and natural-day/account context changes clear the committed pair and show loading for the new scope.
- Dense lane bodies keep the existing scroll height and render only the visible rows plus two rows of overscan. The total call count and all row positions remain intact; scrolling exposes every call without sampling, while offscreen bars and grid lines do not add DOM work during live updates.
- Snapshot cleanup runs as a shutdown-cancellable background task after HTTP readiness, first after 60 seconds and then every 60 seconds. It uses the database pressure gate, a non-blocking P2 admission, and a non-blocking materialization-lock check; busy passes are skipped. Each pass handles at most 48 explicit releases and reserves at least 16 of its 64 candidate slots for the periodic database scan, so a saturated release queue cannot starve reclamation of overflowed or expired rows. It reads the active-token registry after scanning, so a token registered during the scan is either absent from that pass's candidates or present in the active set before deletion. Cleanup deletes expired/non-active tokens by equality, allowing old-process rows to be reclaimed without a schema change or holding the materialization lock across database operations.
- Today's dashboard activity revision triggers an authoritative timeline refresh; a bounded polling refresh keeps bars current between revisions. Yesterday remains HTTP-only.
- The first HTTP snapshot is allowed even when the live SSE connection is unavailable. After a successful snapshot, disconnection freezes live extension until reconnect; an initial failure remains an unavailable state inside the timeline surface.

## Verification Evidence

- Rust timeline maintenance tests passed on the real HTTP handler. Two clients each completed 150 traversals over a populated two-record snapshot using two pages per traversal, received no capacity rejection, released all 300 snapshots, and left no durable snapshot rows after cleanup. With the release queue saturated at 1,024 tokens, one simulated overflow stayed out of the queue and was reclaimed along with 20 orphan rows over two scan passes; each cleanup batch remained bounded at 64 candidates.
- The opt-in long E2E can be repeated on the testbox with `E2E_BASE_URL=http://127.0.0.1:60083 bun run test:e2e:timeline-capacity`. The five-minute two-client run passed with 20 desktop and 21 mobile traversals, 82 page requests, 41 snapshot releases, zero HTTP/release/SSE errors, one maximum in-flight traversal per client, and a minimum traversal spacing of 15.05 seconds. Each client recorded 315 revision triggers and 315 SSE live messages, exceeding the full-run assertion threshold. Result artifacts are under `/srv/codex/agents/01a0d6c9-9cca-7eb2-b240-4341071f5997/timeline-capacity-refresh-20260928-1238/web/test-results/dashboard-timeline-capacity-repair-20260928/`.
- Isolated production-shaped desktop and mobile runs each completed two traversals and released both snapshots with zero request errors. The maximum observed long task was 123 ms on desktop and 86 ms on mobile during page readiness; the delayed refresh phase measured 0 ms on both. Both runs retained the last committed chart while the delayed refresh was pending and recovered after the live revision.
- The dense desktop/mobile and stale desktop Storybook captures match their canonical Spec images. The pending desktop capture has only a small raster difference in the plot/legend area; visual inspection found no change to content, layout, styling, or interaction. Existing canonical assets remain the visual reference.
- Targeted Storybook coverage passed for 10 timeline states. The five timeline maintenance Rust tests, including the saturated-queue and populated multi-page cases, passed. Web refresh tests, typecheck, lint, and the complete web unit suite passed; the long E2E passed both capacity-scheduler and desktop/mobile performance scenarios.

## Remaining Gaps

- Full CI and formal review convergence are delivery gates after the topic branch is published.
- The local Storybook review covers representative sparse, dense, empty, unavailable, desktop, and mobile states, with TTFT overlaid in the invocation plot, the X-axis at the bottom, at least 4 visual lanes, adaptive 1–16px lane heights, no gap at 1px or 2px, and a 1 CSS pixel gap above 2px. Pagination removes the former global 2,000-record cutoff; a failed traversal remains an explicit unavailable or stale state inside the new chart surface.
- User-facing axis labels are `并发调用数` / `Concurrent calls` and `TTFT`; internal coordinate labels are excluded from the rendered surface. The timeline header no longer displays the visible-window Invocation total.
- Invocation bars render without embedded text; per-invocation TTFT remains available in the bar title/ARIA label without an extra visual marker. Short and unknown-duration calls use an 8px minimum click width so the invocation remains a horizontal bar.
- The chart frame remains 320px on desktop and 336px on compact viewports. Lane height adapts before overflow; only the lane body scrolls once 1px lanes still exceed the available plot, while the calls axis stays pinned and scroll-synchronized with the lane body. First load of an overflowing view starts at the bottom, and a same-view refresh preserves the user's scroll position within the updated bounds.
- Normal concurrency maps each invocation row to its numeric concurrent-call value (`lane + 1`) on the linear Y axis; the first assigned row is closest to the zero baseline. The hover line carries a chart-local floating tooltip with time, parallel, running, and queued counts; the tooltip does not add a layout row.
- Fresh full CI and Tier 3 current-head review convergence remain required before Step 5C Ready.

## Related Changes

- [ADR 0019: Dashboard timeline replacement contract](../../adr/0019-dashboard-timeline-replacement-contract.md)

## References

- `./SPEC.md`
- `./HISTORY.md`
