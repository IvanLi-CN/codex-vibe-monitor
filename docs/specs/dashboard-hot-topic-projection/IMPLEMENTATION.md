# Dashboard Hot Topic 内存投影与 SSE 稳定性实现状态

> 当前有效规范仍以 `./SPEC.md` 为准；这里记录实现覆盖、交付进度与 rollout 相关事实，避免这些细节散落到 PR / Git 历史里。

## Current Status

- Implementation: 未开始
- Lifecycle: active
- Catalog note: activity、summary 与 network 已有 typed 基础；本主题三条 hot topic 尚未完成迁移

## Coverage / rollout summary

- working-conversations 仍可通过通用 Prompt Cache builder 完整 hydrate。
- open-range parallel-work 仍可执行当前范围 exact query。
- open-window timeseries 仍可进入通用 timeseries builder。
- per-topic hot health 与端到端 Dashboard bundle 性能门禁尚未建立。

## Remaining Gaps

- 建立 topic class 的穷举类型边界。
- 为三条 hot topic 实现 typed projection/materializer 与 bounded recovery。
- 稳定 activity `recentLimit` SSE descriptor。
- 增加 System Status health、浏览器视口证据与线上 A/B 验收。

## Related Changes

- None

## References

- `./SPEC.md`
- `./HISTORY.md`
- `../high-frequency-runtime-data-plane/IMPLEMENTATION.md`
