# Dashboard 对外调用时间线 实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: implemented
- Lifecycle: active
- Catalog note: dashboard / invocation timeline / TTFT

## Implementation Coverage

- `REQ-DIT-001`, `REQ-DIT-004`: `src/api/slices/invocations_and_summary.rs`, `src/maintenance/hourly_rollups.rs`.
- `REQ-DIT-002`, `REQ-DIT-003`, `REQ-DIT-005`, `REQ-DIT-006`: `web/src/hooks/useInvocationTimeline.ts`, `web/src/features/dashboard/DashboardInvocationTimeline.tsx`, `web/src/features/dashboard/DashboardActivityOverview.tsx`.
- API normalization: `web/src/lib/api/core-foundation.ts`, `web/src/lib/api/feature-clients.ts`, `web/src/lib/api/types.ts`.
- Verification: Rust overlap tests, web API normalization tests, web lane assignment tests, typecheck, Rust check, and responsive visual evidence.

## Coverage / rollout summary

- The endpoint is additive and read-only. Existing aggregate charts remain the explicit offline and over-limit fallback.
- Today's dashboard activity revision triggers an authoritative timeline refresh; a bounded polling refresh keeps bars current between revisions. Yesterday remains HTTP-only.

## Remaining Gaps

- Full CI and formal review convergence are delivery gates after the topic branch is published.
- The local Storybook review covered the representative dense state at desktop and the source-managed mobile viewport configuration, with TTFT overlaid in the invocation plot, the X-axis at the bottom, at least 4 visual lanes, adaptive 8–16px lane heights, and a 1 CSS pixel gap between adjacent lanes; production traffic remains subject to the 2,000-record fallback.
- User-facing axis labels are `调用` / `Calls` and `TTFT`; internal coordinate labels are excluded from the rendered surface.
- Invocation bars render without embedded text; per-invocation TTFT remains available in the bar title/ARIA label without an extra visual marker. Short and unknown-duration calls use an 8px minimum click width so the invocation remains a horizontal bar.
- High-concurrency layout keeps the 320px desktop / 336px compact frame fixed, scrolls only the lane body, and pins the X-axis and TTFT scale so 190 lanes do not enlarge the chart.
- Normal concurrency maps each invocation row to its numeric call-count value (`lane + 1`) on the linear Y axis; the first assigned row is closest to the zero baseline, with the existing 1px row gap preserved rather than stretched.

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
