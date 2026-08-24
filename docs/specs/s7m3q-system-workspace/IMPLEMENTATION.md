# 系统工作区重构 - Implementation

## Current State

- Canonical spec: `docs/specs/s7m3q-system-workspace/SPEC.md`
- Status: 实现中

## Implementation Summary

- 新增 `system` 顶层工作区与四个子页：`状态 / 任务 / 设置 / 代理`。
- 顶层导航由 `设置` 改为 `系统`，旧 `#/settings` 改为兼容跳转。
- 新增系统状态接口与系统后台任务记录接口。
- 系统任务列表保持既有 page/pageSize 语义，并增加以 `(startedAt, id)` 为锚点的 additive cursor 翻页；查询直接比较 UTC ISO 时间文本，schema 同时提供默认时间排序和 task/status 组合筛选排序索引。
- 原 settings 页按职责拆分为通用设置页与 forward-proxy 页，同时继续复用现有设置数据模型与写接口。
- 系统状态页 raw 统计已切换为真实磁盘文件口径，并拆分为 `raw / request / response` 三组指标。
- raw 指标已改为持久化增量快照：legacy path 由有界 cursor 补齐，启动时一次性回填旧 invocation/attempt owner link，新写入通过 response/request blob link 增量发现；状态页请求只读快照，不再枚举全部 raw 路径或逐文件读取元数据。pressure defer 或后台失败状态保留在内存 health override，避免诊断写抢占 SQLite。
- System Status 在启动时完成 last-good snapshot hydration，后台以最长 60 秒 cadence 维护 SQL 与文件体积结果；HTTP handler 只克隆未超过 60 秒的内存响应，TTL 到期、失效标记或刷新失败都不在请求路径执行 SQLite、`Path::exists` 或目录扫描。刷新在边界内失败时保留 last-good，超过边界使用端点 unavailable 契约。
- 系统状态页布局已从 12 张等权卡片重构为“项目磁盘总览 + 数据库记录概况 + 归档与逻辑体量”。
- 系统状态接口补充 `liveInvocationsCount` 与 `completedArchiveBatchesCount`，用于解释 live 数据库与归档来源。
- 系统状态页已把 `raw payload` 解释前置到数字旁：主读数旁展示项目总量公式，`raw payload` 总量显式标成“并集总量”，request / response 显式标成“侧向拆分”。
- `raw payload 聚焦` 已改成“总量卡 + request 行 + response 行”的纵向层级，去掉窄列中的并排四小卡，避免 request-heavy 场景下数字区被长说明挤压变形。
- 总览首屏已进一步改成顺序流：主读数、项目级 breakdown、`raw payload 聚焦` 依次堆叠，避免左右上半区高度失衡导致的巨大空白。

## Quality Gates

- `bash .github/scripts/run-backend-tests.sh`
- `cd web && bun run test`
- `cd web && bun run build`
- `cd web && bun run build-storybook`

## Disposition

- `spec_disposition=create`
- `project_doc_disposition=none`
- `solution_disposition=none`

## Memory Diagnostics

- `MemoryDiagnosticsRuntime` 在 runtime 启动后执行一次采样，之后每 30 秒采样一次；采样只访问 proc/cgroup 文件和现有内存容器，不增加 System Status 的 SQLite 读。
- 已知组件估算包含 terminal hub/journal、runtime store、Dashboard cache、long-term interval、prompt/network/routing cache、raw writer occupancy 与 SQLite writer queue。timeseries staging 复用 terminal hub pending bytes，避免重复计算。
