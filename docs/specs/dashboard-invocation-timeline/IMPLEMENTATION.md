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
- Dense lane bodies preserve 8–16px bar heights and a fixed 1px gap inside the existing 320px desktop / 336px compact frame. Once the minimum height no longer fits, only the lane body scrolls; the calls axis stays synchronized, and virtualization renders visible rows plus two rows of overscan. The total call count and row positions remain intact, with no sampling or horizontal scrolling.
- Snapshot cleanup runs as a shutdown-cancellable background task after HTTP readiness, first after 60 seconds and then every 60 seconds. It uses the database pressure gate, a non-blocking P2 admission, and a non-blocking materialization-lock check; busy passes are skipped. Each pass handles at most 48 explicit releases and reserves at least 16 of its 64 candidate slots for the periodic database scan, so a saturated release queue cannot starve reclamation of overflowed or expired rows. It reads the active-token registry after scanning, so a token registered during the scan is either absent from that pass's candidates or present in the active set before deletion. Cleanup deletes expired/non-active tokens by equality, allowing old-process rows to be reclaimed without a schema change or holding the materialization lock across database operations.
- Today's dashboard activity revision triggers an authoritative timeline refresh; a bounded polling refresh keeps bars current between revisions. Yesterday remains HTTP-only.
- The first HTTP snapshot is allowed even when the live SSE connection is unavailable. After a successful snapshot, disconnection freezes live extension until reconnect; an initial failure remains an unavailable state inside the timeline surface.

## Verification Evidence

- The timeline release and cleanup contracts passed six targeted Rust tests against the real HTTP handler, covering two-client capacity reuse, idempotent release, bounded cleanup, concurrent materialization, and a saturated release queue.
- The five-minute testbox capacity run passed for two production Demo clients. The desktop client completed 20 traversals, 40 page requests, and 20 releases; the mobile client completed 21 traversals, 42 page requests, and 21 releases. Each saw 315 demo revision triggers and live SSE messages, zero HTTP/release/SSE errors, at most one traversal in flight, and minimum traversal spacing of 15,049.5 ms.
- The corrected dense timeline was exercised with 550 calls at `1440x1000` and `393x852`. Real DOM measurements confirmed every visible call bar is 8–16 CSS px high, adjacent bars have exactly 1 CSS px between them, the lane body scrolls, and only viewport rows plus overscan are rendered. Chart activation Long Tasks peaked at 86 ms on desktop and 72 ms on mobile; delayed refresh measured 0 ms in both viewports. The five-minute run separately verified that completed snapshots are released and refresh traversal spacing stays at least 15 seconds.
- Five Storybook screenshots were captured at `1440x1024` and `393x852`, compared against the corresponding Spec assets, and confirmed by the owner. The accepted current images are stored in `assets/invocation-timeline-desktop.png`, `assets/invocation-timeline-tooltip-left-edge.png`, `assets/invocation-timeline-tooltip-right-edge.png`, `assets/invocation-timeline-dense.png`, and `assets/invocation-timeline-mobile.png`. The dense comparison shows the intended change from a compressed band to readable rows; the 320/336 px frame and 32 px source-managed margins remain fixed.
- The timeline component unit tests passed (19 tests), Storybook interaction tests passed (14 tests), `typecheck:web`, the standard production build, the production Demo build, and focused lint passed. A five-measurement production Dashboard render-performance run passed with a 141 ms maximum across data-ready, update, and sustained-update phases, below the 200 ms limit. Current-head CI and Tier 3 review remain required for delivery.

## Delivery Gates

- Current-head CI, empirical acceptance including refreshed visual evidence, and Tier 3 review are required to reach Step 5C Ready.

## Related Changes

- [ADR 0019: Dashboard timeline replacement contract](../../adr/0019-dashboard-timeline-replacement-contract.md)

## References

- `./SPEC.md`
- `./HISTORY.md`
