# 系统工作区重构 - History

## Key Decisions

- 2026-06-22: 顶层 `设置` 改为 `系统`，旧 `#/settings` 只保留兼容跳转。
- 2026-06-22: `系统/任务` 首版记录系统后台任务运行摘要，不直接复用账号池维护事件。
- 2026-06-22: `系统/状态` 中“非成功数”按 `status != success` 统计；与现有 summary 的 success-like 口径不同，页面需显式标注。
- 2026-06-22: `已归档 body` 首版按 completed invocation archive batches 的 archived rows/file size 统计。
