# Dashboard 对外调用时间线 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- The timeline is the replacement read model for the screenshot's natural-day “今日/昨日 + 次数” activity chart. Existing aggregate metrics and PWA snapshot formats remain compatible outside this replacement surface.

## Replacements / Background

- Replaces the in-scope natural-day “次数” chart presentation with invocation bars. The old aggregate chart is not a fallback for this surface; it remains only for metrics and ranges outside this topic. The timeline reads existing Invocation records directly, so no conversion exception is part of this topic.

## Related Changes

- None

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
