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

## Current Delivery Facts

- The accepted implementation includes process-local runtime observation, separate live dispatcher and admission wait lists, stable persisted task colors, a restart-safe execution/deferral timeline, and bounded revision-based reads, alongside the 37-entry capability catalog, safe schedule editing, reset-to-default semantics, combined filters, and responsive detail views.
- The catalog computes default policy metadata without writing schedule overrides. Existing unsupported overrides remain readable and require an explicit reset; `enabled` is preserved when overrides are cleared.
- Legacy run-history request timestamps and durations retain their previous meanings; the implementation does not infer actual execution start times from them.
- The previously confirmed mock-only desktop and mobile evidence is stored in `docs/specs/task-operations/assets/`. Candidate images for this change remain pending owner confirmation and are not yet canonical Spec assets.

## Related Changes

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/adr/0026-durable-task-execution-and-deferral-timelines.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`
- `docs/specs/task-operations/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
