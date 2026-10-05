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
- Live runtime and timeline data use dedicated SSE topics: snapshots seed the page and task-observation changes publish bounded runtime updates and revision deltas. The browser advances the visible clock between events and presents connecting, reconnecting, and disabled states; after the observation grace period it freezes open state and labels it unknown.
- The implemented task-detail chart region combines recent-100 run metrics and Retention's seven-day backlog with Tabs, rendered empty chart frames and persistent legends. Per-task capabilities separate complete pending population, discovered eligible candidates and committed work; absent measures remain absent.
- The two existing workload views use the shared segmented control and short labels “次数 / 时间”. The latter denotes Retention's seven-day backlog; this naming does not introduce another run-hour filtering mode. Mock runs conserve pending work across arrivals and committed processing, retain zero-commit and partial-commit failures, and do not infer processing from a skipped attempt.
- The owner requires the workload-chart header to place “运行趋势” at the left and the view Tabs at the right of the same row across desktop and mobile. Keeping only the Tab labels on one row does not satisfy this placement requirement; the heading and control must share the row to preserve chart space.
- The run metric view uses overlapping areas sharing a zero baseline. Where the same-unit candidate sets are nested, visible bands correspond to C, D−C and P−D while boundaries and Tooltip values remain C, D and P. Mixed units or unproven containment do not authorize difference bands or overall progress.
- Workload samples use the asynchronous maintenance recorder, remain independent of page lifetime, and protect each task's latest 100 attempts and unconfirmed running sample. Historical attempts retain their identity while unsupported metrics stay unknown. Detail SSE carries revisioned workload snapshots; stale sequences are ignored and recorder gaps are shown.
- Root and child in-memory overlays retain active attempts and prune previous terminal samples when a new attempt begins. Coverage repair records each successfully committed bucket before continuing; later failure or cancellation preserves those confirmed counts in its final asynchronous snapshot.
- Processing speed uses at most 20 complete ended attempts and actual wall-clock span. Backlog estimates require at least five fresh same-range exact snapshots within 24 hours, a 60-second minimum span, positive net decline, enabled task state, and no coverage gap. Skips contribute zero only to the rate window and do not fabricate chart metrics or actual start times.
- The public response additions and maintenance-store workload table are forward-compatible; the API and durable-state compatibility impact is minor. Existing request-time and duration meanings, task configuration, and stored colors remain unchanged.

## Current Delivery Facts

- The accepted implementation includes process-local runtime observation, separate live dispatcher and admission wait lists, stable persisted task colors, a restart-safe execution/deferral timeline, bounded revision-based reads, and durable task workload samples, alongside the 37-entry capability catalog, safe schedule editing, reset-to-default semantics, combined filters, and responsive detail views.
- The catalog computes default policy metadata without writing schedule overrides. Existing unsupported overrides remain readable and require an explicit reset; `enabled` is preserved when overrides are cleared.
- Legacy run-history request timestamps and durations retain their previous meanings; the implementation does not infer actual execution start times from them.
- The owner confirmed the mock-only desktop/mobile timeline and SSE connection-state evidence on 2026-10-02. Canonical assets are stored in `docs/specs/task-operations/assets/`; the mobile capture keeps the 12-hour chart compact without row labels.
- The owner confirmed all six workload-chart rectification screenshots on 2026-10-04. The accepted Storybook evidence replaces the earlier workload images and covers shared single-row Tabs, Retention's time view, mobile chart space, hidden-pending rescaling, and a running task without counters.
- The owner confirmed six header-alignment screenshots on 2026-10-05 and authorized Spec and PR reuse. They replace the prior canonical images and demonstrate the heading and view Tabs sharing one row with opposite-edge alignment on desktop and mobile.
- PR #1074's reconnect Storybook fixture allows 750 ms before the simulated disconnect while preserving its connection-state assertions and timer cleanup. The test-only correction passed current-head CI at `62354bb8`; it does not change production SSE behavior or the accepted workload-chart evidence.

## Related Changes

- [PR #1074: durable workload trends and evidence-based estimates](https://github.com/IvanLi-CN/codex-vibe-monitor/pull/1074)

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/adr/0026-durable-task-execution-and-deferral-timelines.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`
- `docs/specs/task-operations/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
