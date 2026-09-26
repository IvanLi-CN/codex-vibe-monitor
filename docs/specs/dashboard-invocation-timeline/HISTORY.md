# Dashboard 对外调用时间线 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- Added as an additive dashboard read model. Existing aggregate metrics and PWA snapshot formats remain compatible.

## Replacements / Background

- Replaces the natural-day “次数” chart presentation with invocation bars while retaining the aggregate chart as a bounded-data and offline fallback.

## Related Changes

- None

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
