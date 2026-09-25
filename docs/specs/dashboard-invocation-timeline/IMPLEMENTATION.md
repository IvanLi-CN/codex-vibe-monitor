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
- The local Storybook review covered the representative dense state at desktop and the source-managed mobile viewport configuration; production traffic remains subject to the 2,000-record fallback.

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
