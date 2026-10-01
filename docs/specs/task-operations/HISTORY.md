# 任务运维运行观测与生效计划 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- The topic is active. It extends the existing maintenance task API and page while preserving legacy task fields and maintenance-history storage.
- Runtime observations are intentionally ephemeral. A process restart clears active runs; historical task rows remain readable from the maintenance database.
- The additive runtime and catalog fields are compatible with consumers that ignore unknown JSON properties. PATCH clients using existing `enabled`-only requests retain their behavior.

## Replacements / Background

- ADR 0024 records the accepted decision to separate actual execution observation from durable history and to centralize effective schedule policy in a capability catalog.
- The diagnostic solution records the root cause: worker defaults were not present in the database projection, while the detail page rendered an empty override as a fixed interval.

## Current Delivery Facts

- The accepted implementation adds process-local runtime observation, a 37-entry capability catalog, safe interval/UTC-cron editing for the approved task set, reset-to-default semantics, the running-task section, combined filters, and responsive detail views.
- The catalog computes default policy metadata without writing schedule overrides. Existing unsupported overrides remain readable and require an explicit reset; `enabled` is preserved when overrides are cleared.
- Confirmed mock-only desktop and mobile evidence is stored in `docs/specs/task-operations/assets/` and linked from `SPEC.md`.

## Related Changes

- `docs/adr/0024-task-runtime-observation-and-effective-schedules.md`
- `docs/solutions/maintenance/task-schedule-and-running-observation.md`
- `docs/specs/task-operations/assets/version-impact-record.json`

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
