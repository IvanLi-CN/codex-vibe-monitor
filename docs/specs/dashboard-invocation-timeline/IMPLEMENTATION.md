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
- Verification: Rust overlap and background-cleanup tests, web timeline snapshot/window-state tests, web API normalization tests, web lane assignment and layout tests, Storybook interaction coverage, typecheck, Rust check, and responsive visual evidence.

## Coverage / rollout summary

- The endpoint is additive and read-only. It captures invocation and attempt row watermarks, freezes the folded records and runtime overlay in one read transaction, then publishes the bounded result into short-lived SQLite snapshot rows with a short `BEGIN IMMEDIATE` transaction using bounded multi-row batches. Subsequent requests read only those rows through the opaque cursor, so mutable source rows cannot drift between pages without holding the writer lock during the full scan. Snapshot entries expire after 30 minutes; a bounded cache rejects new first-page anchors when full instead of evicting a cursor that may still be in use.
- The timeline request carries the selected natural-day UTC bounds separately from the viewport. The server rejects scopes and viewports wider than 24 hours, clamps persisted and live boundary overlap to the preceding 24 hours, and keeps the existing 30-day duration safety check separate from query range selection. This prevents a short viewport from scanning a 30-day history while preserving long terminal durations for calls that begin inside the selected day.
- Each materialized snapshot has explicit row and serialized-payload budgets, and the active cache has aggregate row and byte budgets. Exceeding any budget returns the timeline's unavailable state and rolls back the partial snapshot; it never truncates the result, samples rows, or mounts the legacy aggregate chart.
- The target `totalCount` path mounts `DashboardInvocationTimeline` without a legacy chart prop. Loading, unavailable, stale, disconnected, and pagination errors remain inside the timeline surface; non-target metrics and ranges keep the aggregate chart.
- The live hook traverses every page with one `asOf`, merges records by `(invokeId, occurredAt)`, coalesces revisions while a request is in flight, and retains the last good snapshot when a refresh fails.
- The live hook keeps the requested viewport separate from one atomically committed `{data, window}` snapshot. Automatic live-window advances continue rendering that committed pair until the replacement succeeds; a failed refresh leaves it visible and stale. User zoom/pan and natural-day/account context changes clear the committed pair and show loading for the new scope.
- Snapshot cleanup runs as a shutdown-cancellable background task after HTTP readiness, first after 60 seconds and then every 60 seconds. It uses the database pressure gate, a non-blocking P2 admission, and a non-blocking materialization-lock check; busy passes are skipped. Each pass scans at most 64 candidate token keys before reading the active-token registry, so a token registered during the scan is either absent from that pass's candidates or present in the active set before deletion. Cleanup deletes expired/non-active tokens by equality, allowing old-process rows to be reclaimed without a schema change or holding the materialization lock across database operations.
- Today's dashboard activity revision triggers an authoritative timeline refresh; a bounded polling refresh keeps bars current between revisions. Yesterday remains HTTP-only.
- The first HTTP snapshot is allowed even when the live SSE connection is unavailable. After a successful snapshot, disconnection freezes live extension until reconnect; an initial failure remains an unavailable state inside the timeline surface.

## Remaining Gaps

- Full CI and formal review convergence are delivery gates after the topic branch is published.
- The local Storybook review covers representative sparse, dense, empty, unavailable, desktop, and mobile states, with TTFT overlaid in the invocation plot, the X-axis at the bottom, at least 4 visual lanes, adaptive 1–16px lane heights, no gap at 1px or 2px, and a 1 CSS pixel gap above 2px. Pagination removes the former global 2,000-record cutoff; a failed traversal remains an explicit unavailable or stale state inside the new chart surface.
- User-facing axis labels are `并发调用数` / `Concurrent calls` and `TTFT`; internal coordinate labels are excluded from the rendered surface. The timeline header no longer displays the visible-window Invocation total.
- Invocation bars render without embedded text; per-invocation TTFT remains available in the bar title/ARIA label without an extra visual marker. Short and unknown-duration calls use an 8px minimum click width so the invocation remains a horizontal bar.
- The chart frame remains 320px on desktop and 336px on compact viewports. Lane height adapts before overflow; only the lane body scrolls once 1px lanes still exceed the available plot, while the calls axis stays pinned and scroll-synchronized with the lane body. First load of an overflowing view starts at the bottom, and a same-view refresh preserves the user's scroll position within the updated bounds.
- Normal concurrency maps each invocation row to its numeric concurrent-call value (`lane + 1`) on the linear Y axis; the first assigned row is closest to the zero baseline. The hover line carries a chart-local floating tooltip with time, parallel, running, and queued counts; the tooltip does not add a layout row.

## Related Changes

- [ADR 0019: Dashboard timeline replacement contract](../../adr/0019-dashboard-timeline-replacement-contract.md)

## References

- `./SPEC.md`
- `./HISTORY.md`
