# 系统工作区重构 - History

## Key Decisions

- 2026-06-22: 顶层 `设置` 改为 `系统`，旧 `#/settings` 只保留兼容跳转。
- 2026-06-22: `系统/任务` 首版记录系统后台任务运行摘要，不直接复用账号池维护事件。
- 2026-06-22: `系统/状态` 中“非成功数”按 `status != success` 统计；与现有 summary 的 success-like 口径不同，页面需显式标注。
- 2026-06-22: `已归档 body` 首版按 completed invocation archive batches 的 archived rows/file size 统计。
- 2026-06-22: `系统/状态` raw payload 改为按 request / response 实际文件路径的磁盘字节数统计，`raw payload` 总量使用去重后的文件并集，不再只看 response 侧逻辑大小。
- 2026-06-23: `系统/状态` 布局改为“实际磁盘占用总览 + 数据库记录概况 + 归档与逻辑体量”，以项目级磁盘总量作为首屏主读数。
- 2026-06-23: `系统/状态` 新增 `liveInvocationsCount` 与 `completedArchiveBatchesCount`，用于解释 live 记录与归档批次来源。
- 2026-06-23: `系统/状态` 把 `raw payload` 的解释从尾注前移到指标本身：总量固定标记为“并集总量”，request / response 固定标记为“侧向拆分”，并在主读数旁直接展示项目总量公式。
- 2026-06-23: `系统/状态` 的 `raw payload 聚焦` 改成“总量卡 + request 行 + response 行”的纵向结构，避免 request-heavy 场景在窄列中出现四小卡挤压变形。
- 2026-06-23: `系统/状态` 顶部总览从左右并排重排为顺序流，先展示主读数与项目级 breakdown，再展示 `raw payload 聚焦`，移除大面积无信息留白。
- 2026-07-15: `系统/状态` 中 live invocation 的 success/non-success 计数改为跟随 success-like 口径；未来新写入的 `warning_success` 计入 success，不再落入 non-success。
